# Gcli-Nexus

> High-performance Gemini CLI reverse proxy that talks to the raw Cloud Code Gemini endpoints while presenting Gemini-native responses.

## Highlights

- **Gemini CLI parity**: accepts the same `/v1beta/models/{model}:generateContent` and `:streamGenerateContent` payloads that the official `geminicli` uses and keeps the native Cloud Code URL/response envelope intact upstream.
- **Gemini-native output**: normalizes CLI envelopes (including SSE events) into the standard Gemini `candidates/usageMetadata/modelVersion` shape so existing dashboards and SDKs can consume the responses directly.
- **Credential pool with actor scheduling**: a `ractor`-driven worker manages Google OAuth credentials stored in SQLite, separates “big” and “tiny” model queues, cools down projects that hit 429, and refreshes tokens only when a credential is near expiry or fails authentication.
- **Operable out of the box**: `.env` configuration via Figment/dotenvy, WAL-enabled SQLite (`data.db`) that bootstraps automatically, structured tracing, and a `mimalloc` global allocator for predictable latency.

## Quick start

### Prerequisites

- Rust 1.78+ (Edition 2024) with `cargo` and a recent `sqlx-cli` compatible SQLite (libsqlite3) on the host.
- Google Cloud projects that already have Gemini CLI access; export each account as the JSON blob that contains `client_id`, `client_secret`, `refresh_token`, `project_id`, etc.

### Bootstrapping

1. Copy the sample environment file and fill in secrets:
   ```bash
   cp .env.example .env
   # edit NEXUS_KEY plus (optionally) DATABASE_URL, BIGMODEL_LIST, PROXY...
   ```
2. Drop every Gemini credential JSON into the folder referenced by `CRED_PATH` (default `./credentials`). On startup the actor will normalize, refresh, and persist them into SQLite. Additions today require a restart to be ingested.
3. Launch the proxy (cargo or prebuilt binary):
   ```bash
   cargo run --release
   # or, if you already built/downloaded the binary, run it directly
   ./gcli-nexus
   ```
   The server binds `0.0.0.0:8000`. Logs reveal how many credentials were activated and whether a proxy is in use.

For production deployments build the binary once (`cargo build --release`) and run it under your supervisor (systemd, container, etc.). `MiMalloc` is compiled in automatically; no extra tuning is required.

## Configuration

| Env var                              | Required | Default            | Description                                                                                                           |
| ------------------------------------ | -------- | ------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `NEXUS_KEY`                          | Yes      | _none_             | Shared secret checked on every request via `x-goog-api-key`, `Authorization: Bearer`, or `?key=`.                     |
| `DATABASE_URL`                       | No       | `sqlite://data.db` | SQLite DSN; the actor enables WAL and creates the file/migrations automatically.                                      |
| `LOGLEVEL`                           | No       | `info`             | Tracing level (`error`, `warn`, `info`, `debug`, `trace`). `RUST_LOG` still works as a fallback.                      |
| `BIGMODEL_LIST`                      | No       | `[]`               | JSON array of model names treated as “big”. They get their own queue/cooldown bucket to avoid starving lighter chats. |
| `CRED_PATH`                          | No       | unset              | Directory that is scanned once during startup for credential JSON; leave unset to rely purely on SQLite contents.     |
| `REFRESH_CONCURRENCY`                | No       | `10`               | Number of concurrent Google refresh jobs buffered in the background worker.                                           |
| `PROXY`                              | No       | unset              | Outbound HTTP proxy applied to both the Gemini caller and the OAuth refresh client (supports HTTP/SOCKS).             |
| `DATABASE_URL`, `PROXY`, `CRED_PATH` | —        | —                  | Accept absolute or relative paths; Figment merges `.env` values automatically.                                        |

### Credential lifecycle

1. **Ingestion**: Each JSON file is parsed via `GoogleCredential::from_payload`, refreshed immediately, and upserted into SQLite. Duplicate `project_id`s are replaced atomically.
2. **Queues**: Active credentials are pushed into both the “big” and “tiny” queues; requests choose a queue based on whether `model` matches `BIGMODEL_LIST`.
3. **Rate limits**: When a 429 response contains `quotaResetTimeStamp`, the actor parks the credential for that many seconds before putting it back in queue.
4. **Refresh flow**: 401/403 responses trigger `ReportInvalid` → refresh pipeline → DB update → re-enqueue. Failing refreshes disable the credential (status=false).
5. **Persistence**: Because the DB is authoritative, restarts reuse the latest access tokens/expiry timestamps without re-reading every JSON file.

## API usage

### Authentication

- Send `x-goog-api-key: <NEXUS_KEY>` (preferred).
- Or append `?key=<NEXUS_KEY>` to the request URL.

### Generate content (non-streaming)

```bash
curl -X POST http://localhost:8000/v1beta/models/gemini-2.5-pro:generateContent \
  -H "x-goog-api-key: $NEXUS_KEY" \
  -H "Content-Type: application/json" \
  -d '{
        "contents":[{"role":"user","parts":[{"text":"hello from gcli-nexus"}]}]
      }'
```

### Streaming

```bash
curl --no-buffer -X POST \
  http://localhost:8000/v1beta/models/gemini-2.5-pro:streamGenerateContent \
  -H "x-goog-api-key: $NEXUS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"contents":[{"role":"user","parts":[{"text":"stream"}]}]}'
```

### Error semantics

- `401/403` from upstream map to a temporary `502/500` locally after a refresh attempt; the credential is refreshed before reuse.
- `429` returns upstream headers/body untouched while the offending credential cools down.
- `503` with `{"error":"no available credential"}` means all queues are empty or cooling—add more credentials or wait for cooldowns.

## Operations

- **Logging**: Structured tracing goes to stdout; set `LOGLEVEL=debug` for detailed actor logs (queue lengths, refresh states). Use `RUST_LOG` for per-module overrides.
- **Database**: `data.db` lives at the path inside `DATABASE_URL`. WAL mode reduces writer stalls; backup the file periodically if you care about history.
- **Proxying**: Set `PROXY` (e.g. `http://127.0.0.1:1080`) if your network requires outbound proxying; both Gemini traffic and OAuth refresh calls use it.
- **Credential rotation**: Update the JSON file, restart the binary, or seed SQLite manually; the actor upserts by `project_id`.
- **Security**: Treat `.env`, `credentials/*.json`, and `data.db` as sensitive—they contain refresh and access tokens.

## License

This project is distributed under the [MIT License](LICENSE).
