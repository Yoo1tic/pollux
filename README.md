# Pollux

Pollux is a headless, actor-driven Rust reverse proxy that orchestrates AI resources. It serves as a microsecond-level scheduler, transforming raw credential resources into standard Gemini & OpenAI interfaces.

Today it ships with two providers:

- **Gemini CLI (Cloud Code)** → exposes the **Gemini v1beta** API surface (`/geminicli/v1beta/models`, `/geminicli/v1beta/models/{model}:generateContent`, `/geminicli/v1beta/models/{model}:streamGenerateContent`)
- **Codex (ChatGPT backend)** → exposes an **OpenAI Responses API–compatible** surface (`/codex/v1/responses`, `/codex/v1/models`)

It is designed to be **stateless at the edge** and **stateful in SQLite**: credentials can be
ingested dynamically, persisted, and scheduled without restarts.

## Highlights

- **Protocol standardization**: Gemini v1beta + OpenAI Responses API (Codex) behind one service.
- **Actor-based scheduling**: built on `ractor` to keep the hot path lock-free.
- **Credential pool & rotation**: retries, rotation on upstream errors, and queue-based scheduling.
- **Streaming support**: SSE passthrough for both Gemini streaming and Codex streaming.
- **Single binary / Docker**: runs as a small container or `cargo run`.

## Authentication

All **proxy endpoints** require a shared key (`basic.pollux_key` in `config.toml`).

Pollux supports all of the following auth forms:

- Header: `Authorization: Bearer <pollux_key>`
- Query: `?key=<pollux_key>`
- Header: `x-goog-api-key: <pollux_key>` (kept for compatibility with Gemini tooling)

Recommended usage (to avoid confusion):

- **Codex endpoints** (`/codex/*`): prefer `Authorization: Bearer <pollux_key>`
- **Gemini CLI endpoints** (`/geminicli/*`): prefer `?key=<pollux_key>`

OAuth entry/callback endpoints do **not** require the key.

## API Surface

### Gemini (Gemini CLI provider)

| Endpoint                                                 | Method | Auth | Description                                           |
| :------------------------------------------------------- | :----- | :--- | :---------------------------------------------------- |
| `/geminicli/v1beta/models`                               | `GET`  | ✅   | List supported Gemini models.                         |
| `/geminicli/v1beta/openai/models`                        | `GET`  | ✅   | List the same models in OpenAI-style `models` format. |
| `/geminicli/v1beta/models/{model}:generateContent`       | `POST` | ✅   | Unary generateContent.                                |
| `/geminicli/v1beta/models/{model}:streamGenerateContent` | `POST` | ✅   | Streaming generateContent (SSE).                      |
| `/geminicli/resource:add`                                | `POST` | ✅   | Ingest Gemini CLI refresh tokens (0-trust, batch).    |
| `/geminicli/auth`                                        | `GET`  | ❌   | Start Google OAuth (Gemini CLI flow).                 |
| `/oauth2callback`                                        | `GET`  | ❌   | Google OAuth callback handler.                        |

### Codex (OpenAI Responses API–compatible)

| Endpoint               | Method | Auth | Description                                                        |
| :--------------------- | :----- | :--- | :----------------------------------------------------------------- |
| `/codex/v1/models`     | `GET`  | ✅   | List supported Codex models.                                       |
| `/codex/v1/responses`  | `POST` | ✅   | OpenAI Responses API–compatible request/streaming response.        |
| `/codex/resource:add`  | `POST` | ✅   | Ingest Codex refresh tokens (0-trust, batch).                      |
| `/codex/auth`          | `GET`  | ❌   | Start OpenAI OAuth (Codex CLI flow).                               |
| `/auth/callback`       | `GET`  | ❌   | Codex OAuth callback handler (same handler as Codex CLI redirect). |
| `/codex/auth/callback` | `GET`  | ❌   | Alias of `/auth/callback`.                                         |

## Quick Start

### 1) Configure (`config.toml`)

Template: [`config.toml.example`](./config.toml.example)

`pollux` requires a real `config.toml` at runtime (and `basic.pollux_key` must be non-empty).

Minimal example:

```toml
[basic]
listen_addr = "0.0.0.0"
listen_port = 8188
database_url = "sqlite://data.db"
loglevel = "info"
pollux_key = "change-me"
insecure_cookie = false

[providers.geminicli]
model_list = ["gemini-2.5-pro"]

[providers.codex]
model_list = ["gpt-5.2-codex"]
```

`basic.insecure_cookie` defaults to `false` (recommended for HTTPS).
If you access Pollux via plain HTTP (for testing), set it to `true`; otherwise browser OAuth session cookies may not be sent.

### 2) Run

**Option A: [Docker Compose]**

Template: [`docker-compose.yml.example`](./docker-compose.yml.example)

- Copy `docker-compose.yml.example` to `docker-compose.yml`
- Make sure your `config.toml` is mounted to `/app/config.toml`
- Start: `docker compose up -d`

**Option B: Local**

```bash
cargo run --release
```

Server defaults to `0.0.0.0:8188` (configurable).

## Onboarding Credentials

### Gemini CLI (Google)

**Method A: OAuth (browser)**

1. Open `http://localhost:8188/geminicli/auth`
2. Complete Google OAuth
3. You should see `Success` from `/oauth2callback`

**Method B: Refresh token ingestion**

```bash
curl -X POST "http://localhost:8188/geminicli/resource:add?key=change-me" \
  -H "Content-Type: application/json" \
  -d '[{"refresh_token":"1//..."}, {"refresh_token":"2//..."}]'
```

Pollux returns `202 Accepted` + `Success` once accepted; detailed validation outcomes are logged.

### Codex (OpenAI)

**Method A: OAuth (browser)**

1. Open `http://localhost:8188/codex/auth`
2. Complete OpenAI OAuth
3. OpenAI redirects to `http://localhost:1455/auth/callback?...` (Codex CLI default)
4. If Pollux is not listening on `1455`, change the port in the browser address bar to your Pollux port
   (e.g. `8188`) and reload.

**Method B: Refresh token ingestion**

```bash
curl -X POST "http://localhost:8188/codex/resource:add" \
  -H "Authorization: Bearer change-me" \
  -H "Content-Type: application/json" \
  -d '[{"refresh_token":"rt_01..."}, {"refresh_token":"rt_02..."}]'
```

## License

See `LICENSE`. This project is licensed under the GNU Affero General Public License v3.0.
