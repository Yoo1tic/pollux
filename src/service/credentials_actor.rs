use crate::config::CONFIG;
use crate::db::sqlite::CredentialsStorage;
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::service::{GoogleOauthService, RefreshJob};
use crate::service::classifier::{BigModelList, ModelClassifier};

use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::collections::{HashMap, HashSet, VecDeque};
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Unique ID bound 1:1 to a specific credential.
/// Now maps to the database autoincrement id.
pub type CredentialId = i64;

/// Data returned when a credential is checked out from the actor.
/// Only includes fields necessary for making requests.
#[derive(Debug, Clone)]
pub struct AssignedCredential {
    pub id: CredentialId,
    pub project_id: String,
    pub access_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    Big,
    Tiny,
}

impl ModelTier {
    fn from_model(classifier: &BigModelList, model_name: &str) -> Self {
        if classifier.is_big_model(model_name) {
            ModelTier::Big
        } else {
            ModelTier::Tiny
        }
    }

    fn label(&self) -> &'static str {
        match self {
            ModelTier::Big => "big",
            ModelTier::Tiny => "tiny",
        }
    }
}

/// Public messages handled by the credentials actor.
#[derive(Debug)]
pub enum CredentialsActorMessage {
    /// Request one available credential for the given model. Err if none available.
    GetCredential(String, RpcReplyPort<Option<AssignedCredential>>),
    /// Report rate limiting; start cooldown then re-enqueue when complete.
    ReportRateLimit {
        id: CredentialId,
        cooldown: Duration,
        model_name: String,
    },
    /// Report invalid/expired access (e.g. 401/403); refresh then re-enqueue.
    ReportInvalid { id: CredentialId },
    /// Report a credential as banned/unusable; remove from queues and storage.
    ReportBaned { id: CredentialId },

    /// Submit a batch of credentials and trigger one refresh pass for each.
    SubmitCredentials(Vec<GoogleCredential>),

    // Internal messages (sent by the actor itself)
    /// Cooldown has completed; put credential back to the queue if still valid.
    CooldownComplete { id: CredentialId, tier: ModelTier },
    /// Token refresh has completed; update stored credential and re-enqueue if ok.
    RefreshComplete {
        id: CredentialId,
        result: Result<GoogleCredential, NexusError>,
    },
}

/// Handle for interacting with the credentials actor.
#[derive(Clone)]
pub struct CredentialsHandle {
    actor: ActorRef<CredentialsActorMessage>,
}

impl CredentialsHandle {
    /// Request a credential based on target model. Returns error if none available.
    pub async fn get_credential(
        &self,
        model_name: impl AsRef<str>,
    ) -> Result<Option<AssignedCredential>, NexusError> {
        ractor::call!(
            self.actor,
            CredentialsActorMessage::GetCredential,
            model_name.as_ref().to_string()
        )
        .map_err(|e| NexusError::RactorError(format!("GetCredential RPC failed:: {e}")))
    }

    /// Report rate limit; the actor will cool down this credential before reuse.
    pub async fn report_rate_limit(
        &self,
        id: CredentialId,
        model_name: impl AsRef<str>,
        cooldown: Duration,
    ) {
        let _ = ractor::cast!(
            self.actor,
            CredentialsActorMessage::ReportRateLimit {
                id,
                cooldown,
                model_name: model_name.as_ref().to_string()
            }
        );
    }

    /// Report invalid/expired (401/403); the actor will refresh before reuse.
    pub async fn report_invalid(&self, id: CredentialId) {
        let _ = ractor::cast!(self.actor, CredentialsActorMessage::ReportInvalid { id });
    }

    /// Report a credential as permanently banned/unusable; remove it entirely.
    pub async fn report_baned(&self, id: CredentialId) {
        let _ = ractor::cast!(self.actor, CredentialsActorMessage::ReportBaned { id });
    }

    /// Submit new credentials to the actor and trigger refresh for each.
    pub async fn submit_credentials(&self, creds: Vec<GoogleCredential>) {
        let _ = ractor::cast!(
            self.actor,
            CredentialsActorMessage::SubmitCredentials(creds)
        );
    }
}

/// Internal state held by ractor-driven credentials actor
struct CredentialsActorState {
    refresh_tx: mpsc::UnboundedSender<RefreshJob>,

    model_classifier: BigModelList,

    // Scheduling + storage
    bigmodel_queue: VecDeque<CredentialId>,
    tinymodel_queue: VecDeque<CredentialId>,
    creds: HashMap<CredentialId, GoogleCredential>,

    // Runtime state
    cooling_down_big: HashSet<CredentialId>,
    cooling_down_tiny: HashSet<CredentialId>,
    refreshing: HashSet<CredentialId>,

    // Storage layer for credentials persistence
    storage: CredentialsStorage,
}

impl CredentialsActorState {
    fn project_id_of(&self, id: CredentialId) -> Option<&str> {
        self.creds.get(&id).map(|c| c.project_id.as_str())
    }

    fn remove_from_all_queues(&mut self, target: CredentialId) {
        self.bigmodel_queue.retain(|&x| x != target);
        self.tinymodel_queue.retain(|&x| x != target);
    }

    fn clear_from_cooldowns(&mut self, target: CredentialId) -> bool {
        let removed_big = self.cooling_down_big.remove(&target);
        let removed_tiny = self.cooling_down_tiny.remove(&target);
        removed_big || removed_tiny
    }

    fn push_back_all(&mut self, id: CredentialId) {
        self.bigmodel_queue.push_back(id);
        self.tinymodel_queue.push_back(id);
    }

    fn push_back_for_tier(&mut self, id: CredentialId, tier: ModelTier) {
        match tier {
            ModelTier::Big => self.bigmodel_queue.push_back(id),
            ModelTier::Tiny => self.tinymodel_queue.push_back(id),
        }
    }

    fn remove_from_tier_queue(&mut self, target: CredentialId, tier: ModelTier) {
        match tier {
            ModelTier::Big => self.bigmodel_queue.retain(|&x| x != target),
            ModelTier::Tiny => self.tinymodel_queue.retain(|&x| x != target),
        }
    }

    fn queue_len(&self, tier: ModelTier) -> usize {
        match tier {
            ModelTier::Big => self.bigmodel_queue.len(),
            ModelTier::Tiny => self.tinymodel_queue.len(),
        }
    }

    fn cooling_len(&self, tier: ModelTier) -> usize {
        match tier {
            ModelTier::Big => self.cooling_down_big.len(),
            ModelTier::Tiny => self.cooling_down_tiny.len(),
        }
    }

    fn pop_for_tier(&mut self, tier: ModelTier) -> Option<CredentialId> {
        match tier {
            ModelTier::Big => self.bigmodel_queue.pop_front(),
            ModelTier::Tiny => self.tinymodel_queue.pop_front(),
        }
    }
}

/// ractor-based credentials actor
struct CredentialsActor;

#[ractor::async_trait]
impl Actor for CredentialsActor {
    type Msg = CredentialsActorMessage;
    type State = CredentialsActorState;
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _arguments: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let svc = GoogleOauthService::new();
        let refresh_tx = svc.refresh_tx();
        let classifier = BigModelList::new(CONFIG.bigmodel_list.clone());

        let mut bigmodel_queue = VecDeque::new();
        let mut tinymodel_queue = VecDeque::new();
        let mut creds = HashMap::new();
        // Connect to DB and load active credentials. Any failure aborts startup.
        let connect_opts = SqliteConnectOptions::from_str(CONFIG.database_url.as_str())
            .map_err(|e| ActorProcessingErr::from(format!("DB opts parse failed: {}", e)))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .connect_with(connect_opts)
            .await
            .map_err(|e| ActorProcessingErr::from(format!("DB connect failed: {}", e)))?;
        let storage = CredentialsStorage::new(pool);
        storage
            .init_schema()
            .await
            .map_err(|e| ActorProcessingErr::from(format!("DB init schema failed: {}", e)))?;
        let rows = storage
            .list_active()
            .await
            .map_err(|e| ActorProcessingErr::from(format!("DB load active creds failed: {}", e)))?;
        for row in rows {
            let id = row.id as CredentialId;
            let cred: GoogleCredential = row.into();
            bigmodel_queue.push_back(id);
            tinymodel_queue.push_back(id);
            creds.insert(id, cred);
        }
        info!(
            "CredentialsActor started from DB: {} active creds",
            creds.len()
        );
        Ok(CredentialsActorState {
            refresh_tx,
            model_classifier: classifier,
            bigmodel_queue,
            tinymodel_queue,
            creds,
            cooling_down_big: HashSet::new(),
            cooling_down_tiny: HashSet::new(),
            refreshing: HashSet::new(),
            storage,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            CredentialsActorMessage::GetCredential(model_name, rp) => {
                self.handle_get_credential(state, rp, &myself, model_name)
                    .await;
            }
            CredentialsActorMessage::ReportRateLimit {
                id,
                cooldown,
                model_name,
            } => {
                self.handle_report_rate_limit(state, &myself, id, cooldown, model_name);
            }
            CredentialsActorMessage::ReportInvalid { id } => {
                let pid = state.project_id_of(id).unwrap_or("-");
                info!("ID: {id}, Project: {pid}, invalid reported; starting refresh");
                self.handle_report_invalid(state, &myself, id).await;
            }
            CredentialsActorMessage::ReportBaned { id } => {
                self.handle_report_baned(state, id).await;
            }
            CredentialsActorMessage::SubmitCredentials(creds_vec) => {
                self.handle_submit_credentials(state, &myself, creds_vec)
                    .await;
            }
            CredentialsActorMessage::CooldownComplete { id, tier } => {
                let removed = match tier {
                    ModelTier::Big => state.cooling_down_big.remove(&id),
                    ModelTier::Tiny => state.cooling_down_tiny.remove(&id),
                };
                if !removed {
                    return Ok(());
                }
                if state.creds.contains_key(&id) {
                    state.push_back_for_tier(id, tier);
                    info!(
                        "ID: {id}, Cooldown complete; push back {} queue",
                        tier.label()
                    );
                }
            }
            CredentialsActorMessage::RefreshComplete { id, result } => {
                if !state.refreshing.remove(&id) {
                    return Ok(());
                }
                match result {
                    Ok(updated) => {
                        debug!(
                            "ID: {id}, Project: {}, Refresh completed successfully",
                            updated.project_id
                        );
                        state.creds.insert(id, updated.clone());
                        if let Err(e) = state.storage.update_by_id(id, updated.clone(), true).await
                        {
                            warn!("ID: {id}, DB update after refresh failed: {}", e);
                        }
                        state.push_back_all(id);
                    }
                    Err(e) => {
                        error!("ID: {id}, Refresh failed; removing credential: {}", e);
                        state.creds.remove(&id);
                        state.remove_from_all_queues(id);
                        if let Err(e2) = state.storage.set_status(id, false).await {
                            warn!("ID: {id}, DB set_status(false) failed: {}", e2);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl CredentialsActor {
    async fn handle_get_credential(
        &self,
        state: &mut CredentialsActorState,
        reply_port: RpcReplyPort<Option<AssignedCredential>>,
        myself: &ActorRef<CredentialsActorMessage>,
        model_name: String,
    ) {
        let tier = ModelTier::from_model(&state.model_classifier, &model_name);
        debug!(
            "GetCredential start tier={}, queue_len={}, cooling_big={}, cooling_tiny={}",
            tier.label(),
            state.queue_len(tier),
            state.cooling_down_big.len(),
            state.cooling_down_tiny.len()
        );
        while let Some(id) = state.pop_for_tier(tier) {
            let Some(cred_ref) = state.creds.get(&id) else {
                warn!("ID: {id}, Missing credential for id; skipping");
                continue;
            };
            if cred_ref.is_expired() || cred_ref.access_token.is_none() {
                let _ = ractor::cast!(myself, CredentialsActorMessage::ReportInvalid { id });
                continue;
            }
            let token = cred_ref.access_token.as_ref().unwrap().clone();
            debug!(
                "ID: {id}, Project: {}, queue: {}, get credential",
                cred_ref.project_id,
                tier.label()
            );
            let assigned = AssignedCredential {
                id,
                project_id: cred_ref.project_id.clone(),
                access_token: token,
            };
            state.push_back_for_tier(id, tier);
            let _ = reply_port.send(Some(assigned));
            return;
        }
        warn!(
            "No credential available for tier={}, queue_len={}, cooling_big={}, cooling_tiny={}",
            tier.label(),
            state.queue_len(tier),
            state.cooling_down_big.len(),
            state.cooling_down_tiny.len()
        );
        let _ = reply_port.send(None);
    }

    fn handle_report_rate_limit(
        &self,
        state: &mut CredentialsActorState,
        myself: &ActorRef<CredentialsActorMessage>,
        id: CredentialId,
        cooldown: Duration,
        model_name: String,
    ) {
        if !state.creds.contains_key(&id) {
            return;
        }
        let tier = ModelTier::from_model(&state.model_classifier, &model_name);
        state.remove_from_tier_queue(id, tier);
        let inserted = match tier {
            ModelTier::Big => state.cooling_down_big.insert(id),
            ModelTier::Tiny => state.cooling_down_tiny.insert(id),
        };
        if inserted {
            let me = myself.clone();
            tokio::spawn(async move {
                sleep(cooldown).await;
                let _ = ractor::cast!(me, CredentialsActorMessage::CooldownComplete { id, tier });
            });
            info!(
                "ID: {id}, Credential starting cooldown for {} queue, will re-enqueue after cooldown {} secs",
                tier.label(),
                cooldown.as_secs(),
            );
        } else {
            debug!(
                "ID: {id}, tier={} already cooling; queue_len={}, cooling_len={}",
                tier.label(),
                state.queue_len(tier),
                state.cooling_len(tier)
            );
        }
    }

    async fn handle_report_invalid(
        &self,
        state: &mut CredentialsActorState,
        myself: &ActorRef<CredentialsActorMessage>,
        id: CredentialId,
    ) {
        if state.clear_from_cooldowns(id) {
            debug!(
                "ID: {id}, Upgrade: cooldown -> refresh (removed from cooling queues, big_len={}, tiny_len={})",
                state.cooling_down_big.len(),
                state.cooling_down_tiny.len()
            );
        }
        if state.refreshing.contains(&id) {
            debug!("ID: {id}, Already refreshing; skip duplicate");
            return;
        }
        let Some(current) = state.creds.get(&id).cloned() else {
            return;
        };

        state.remove_from_all_queues(id);
        state.refreshing.insert(id);
        let (tx_done, rx_done) = oneshot::channel();
        let job = RefreshJob {
            cred: current,
            respond_to: tx_done,
        };
        match state.refresh_tx.send(job) {
            Ok(_) => {
                let me = myself.clone();
                tokio::spawn(async move {
                    let res = match rx_done.await {
                        Ok(r) => r,
                        Err(e) => Err(NexusError::RactorError(format!(
                            "refresh result channel closed: {}",
                            e
                        ))),
                    };
                    let _ = ractor::cast!(
                        me,
                        CredentialsActorMessage::RefreshComplete { id, result: res }
                    );
                });
                debug!("ID: {id}, Credential refresh enqueued");
            }
            Err(e) => {
                state.refreshing.remove(&id);
                state.push_back_all(id);
                warn!("ID: {id}, Failed to enqueue refresh job: {}", e);
            }
        }
    }

    async fn handle_report_baned(&self, state: &mut CredentialsActorState, id: CredentialId) {
        let project = state
            .project_id_of(id)
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string());
        let removed_cred = state.creds.remove(&id).is_some();
        state.remove_from_all_queues(id);
        let removed_cooldown = state.clear_from_cooldowns(id);
        let removed_refresh = state.refreshing.remove(&id);
        if let Err(e) = state.storage.set_status(id, false).await {
            warn!(
                "ID: {id}, Project: {project}, ban report failed to update DB status: {}",
                e
            );
            return;
        }
        info!(
            "ID: {id}, Project: {project}, credential banned; removed_cred={}, removed_cooldown={}, removed_refreshing={}",
            removed_cred, removed_cooldown, removed_refresh
        );
    }

    async fn handle_submit_credentials(
        &self,
        state: &mut CredentialsActorState,
        _myself: &ActorRef<CredentialsActorMessage>,
        creds_vec: Vec<GoogleCredential>,
    ) {
        for cred in creds_vec.into_iter() {
            let pid = cred.project_id.clone();
            let (tx_done, rx_done) = oneshot::channel();
            let job = RefreshJob {
                cred,
                respond_to: tx_done,
            };
            if let Err(e) = state.refresh_tx.send(job) {
                warn!(
                    "Project: {pid}, failed to enqueue refresh before insert: {}",
                    e
                );
                continue;
            }
            let refreshed = match rx_done.await {
                Ok(Ok(updated)) => updated,
                Ok(Err(e)) => {
                    warn!("Project: {pid}, refresh before insert failed: {}", e);
                    continue;
                }
                Err(e) => {
                    warn!(
                        "Project: {pid}, refresh channel closed before insert: {}",
                        e
                    );
                    continue;
                }
            };
            match state.storage.upsert(refreshed.clone(), true).await {
                Ok(id) => {
                    state.remove_from_all_queues(id);
                    state.creds.insert(id, refreshed);
                    state.push_back_all(id);
                    info!("ID: {id}, Project: {pid}, submitted and activated");
                }
                Err(e) => {
                    warn!("Project: {pid}, upsert failed: {}", e);
                }
            }
        }
    }
}

/// Async spawn of the credentials actor and return a handle.
/// Actor will connect to DB, init schema, and load active credentials.
pub async fn spawn() -> CredentialsHandle {
    let (actor, _jh) = Actor::spawn(Some("CredentialsActor".to_string()), CredentialsActor, ())
        .await
        .expect("failed to spawn CredentialsActor");
    CredentialsHandle { actor }
}
