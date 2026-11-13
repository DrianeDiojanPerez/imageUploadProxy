# Setup

## 1. Create a repo for the images

On GitHub, create a new repository, for example `image-cdn`.

**Private is the better choice.** Reads go through `GET /images/{name}` either
way, so nobody can browse the repo, list what is in it, or walk its history.
Public works too and costs one less API call per view, since you could then
point callers straight at `raw.githubusercontent.com` yourself.

## 2. Create a fine grained token

Go to Settings, Developer settings, Personal access tokens, Fine grained
tokens, and generate a new token.

- Resource owner: the account or org that owns the repo
- Repository access: only select repositories, and pick the repo you just made
- Repository permissions: **Contents** set to **Read and write**

Copy the token, GitHub only shows it once.

## 3. Set the env vars

Copy `.env.example` to `.env` and fill it in:

```bash
cp .env.example .env
```

Every variable is listed in [Configuration](configuration.md).

## 4. Run it

```bash
cargo run
```

The server listens on `0.0.0.0:3000`, or wherever `BIND_ADDRESS` points.
