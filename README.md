# image-upload-proxy

A small Axum service that takes an image upload and stores it, returning a URL
the image can be read back from. The default backend commits the file to a
GitHub repository through the Contents API, and serves it back through this
same service, so the repo can be private and the token never leaves the server.

- [Setup](#setup)
- [Run](#run)
- [Use](#use)
- [Docs](#docs)

## Setup

1. Create a GitHub repo for the images. Private is the better choice.
2. Create a fine grained token for that repo with **Contents** set to **Read and write**.
3. Copy the example env file and fill in `GH_TOKEN`, `GH_OWNER`, and `GH_REPO`:
   ```bash
   cp .env.example .env
   ```

## Run

```bash
cargo run
```

The server listens on `0.0.0.0:3000`, or wherever `BIND_ADDRESS` points.

## Use

Upload an image:

```bash
curl -F "file=@photo.jpg" localhost:3000/upload
```

```json
{ "url": "http://localhost:3000/images/6f1d5f4e-....jpg" }
```

Read it back:

```bash
curl localhost:3000/images/6f1d5f4e-....jpg
```

Delete it:

```bash
curl -X DELETE -H "x-upload-secret: some-secret" localhost:3000/images/6f1d5f4e-....jpg
```

Check the GitHub api budget:

```bash
curl -H "x-upload-secret: some-secret" localhost:3000/usage
```

## Docs

- [Setup](docs/setup.md): the repo, the token, and the first run
- [Configuration](docs/configuration.md): every env var
- [Uploading](docs/uploading.md)
- [Reading an image back](docs/reading.md): the open read route and caching
- [Deleting an image](docs/deleting.md)
- [Checking the api budget](docs/usage.md): `/usage` and how GitHub's limit works
- [Responses](docs/responses.md): every status code
- [Auth](docs/auth.md): the upload secret
- [Logging](docs/logging.md)
- [Testing with Bruno](docs/bruno.md)
- [Adding another storage backend](docs/backends.md)
- [Layout](docs/layout.md): where things live in `src/`
