# image-upload-proxy

A small Axum service that takes an image upload and stores it, returning a URL
the image can be read back from. The default backend commits the file to a
GitHub repository through the Contents API, and serves it back through this
same service, so the repo can be private and the token never leaves the server.

Storage sits behind an `ImageRepository` trait, so the HTTP handler never knows
which backend it is talking to.

## Setup

### 1. Create a repo for the images

On GitHub, create a new repository, for example `image-cdn`.

**Private is the better choice.** Reads go through `GET /images/{name}` either
way, so nobody can browse the repo, list what is in it, or walk its history.
Public works too and costs one less API call per view, since you could then
point callers straight at `raw.githubusercontent.com` yourself.

### 2. Create a fine grained token

Go to Settings, Developer settings, Personal access tokens, Fine grained
tokens, and generate a new token.

- Resource owner: the account or org that owns the repo
- Repository access: only select repositories, and pick the repo you just made
- Repository permissions: **Contents** set to **Read and write**

Copy the token, GitHub only shows it once.

### 3. Set the env vars

Copy `.env.example` to `.env` and fill it in:

```bash
cp .env.example .env
```

| Variable | Default | What it does |
| --- | --- | --- |
| `GH_TOKEN` | required | Fine grained token with Contents read and write |
| `GH_OWNER` | required | The user or org that owns the repo |
| `GH_REPO` | required | The repo, public or private |
| `GH_BRANCH` | `main` | Branch the images are committed to |
| `GH_IMAGE_DIR` | `images` | Where the files sit inside the repo |
| `PUBLIC_BASE_URL` | `http://localhost:3000` | What returned URLs are built from |
| `BIND_ADDRESS` | `0.0.0.0:3000` | Address to listen on |
| `MAX_UPLOAD_MB` | `25` | Largest accepted image, 1 to 100 |
| `ALLOWED_TYPES` | all four | Narrows what an upload may be |
| `CACHE_MAX_AGE` | `31536000` | Seconds a served image may be cached |
| `TIMEZONE` | unset | Your IANA zone, so `/usage` shows local time |
| `USAGE_FILE` | `./usage.json` | Path to keep the call count across restarts |
| `UPLOAD_SECRET` | unset | Guards writing, see Auth below |
| `LOG_FORMAT` | `pretty` | `pretty` or `json` |
| `RUST_LOG` | info | Standard tracing filter |

`PUBLIC_BASE_URL` is what every returned URL is built from, so it has to be the
address callers can actually reach. The default is right for local work and
wrong the moment you deploy.

`ALLOWED_TYPES` can only narrow the built in set of png, jpg, gif, and webp,
since anything else would be stored with a type no browser renders. `jpeg`
counts as `jpg`. `CACHE_MAX_AGE=0` sends `no-store`, which is handy while
debugging.

Every one of these is checked on boot. A `MAX_UPLOAD_MB` of `abc`, an
`ALLOWED_TYPES` naming `bmp`, or a `CACHE_MAX_AGE` of `soon` all stop the
process with a message saying which value was wrong, rather than quietly
falling back to a default you did not ask for.

Exporting them in the shell works just as well, and a variable that is already
exported always beats the one in the file. The three required vars are read at
boot and the process dies right away if any of them is missing.

### 4. Run it

```bash
cargo run
```

The server listens on `0.0.0.0:3000`, or wherever `BIND_ADDRESS` points.

## Uploading

```bash
curl -F "file=@photo.jpg" localhost:3000/upload
```

With auth turned on:

```bash
curl -H "x-upload-secret: some-secret" -F "file=@photo.jpg" localhost:3000/upload
```

A successful upload answers `201` with the URL the image is served from:

```json
{ "url": "http://localhost:3000/images/6f1d5f4e-....jpg" }
```

The image type is taken from the field's content type, falling back to the
filename extension when the client does not send one, and checked against
`ALLOWED_TYPES`. Files are stored as `{GH_IMAGE_DIR}/{uuid}.{ext}` so two
uploads never land on the same path.

## Reading an image back

```bash
curl localhost:3000/images/6f1d5f4e-....jpg
```

`GET /images/{name}` is open, no secret needed, so the URL drops straight into
an `img` tag or a chat client that unfurls links. The uuid in the path is what
keeps a stored image from being found by guessing.

The handler asks GitHub for the file with the token, then answers with the
bytes, an `ETag`, and a `Cache-Control` built from `CACHE_MAX_AGE`, a year by
default.

A name that is not a uuid plus a known extension is turned down as a 404
before any request goes out, which is what stops a path like `../../.env` from
being read back out of the repo.

### Caching

Every stored name carries a uuid, and the bytes behind a uuid never change.
That one fact is what makes the caching here as aggressive as it is: a cached
copy cannot go stale, because changing the image means a new name and a new
URL.

So the answer carries:

```
Cache-Control: public, max-age=31536000, immutable
ETag: "6f1d5f4e-....jpg"
```

`max-age` of a year is the ceiling worth asking for, browsers clamp beyond
that. `immutable` is the part that actually stops the traffic. Without it a
browser still fires a revalidating request on every reload, gets a 304, and
pays the round trip anyway. With it, a reload is served straight from disk
with no request at all.

The `ETag` is the stored name, which means **a revalidation costs nothing**.
When a browser does ask, sending `If-None-Match`, the handler recognises the
name it handed out and answers `304` without calling GitHub at all. That is
worth more here than the saved bytes, since the GitHub API budget is the
scarce thing. Weak tags, comma separated lists, and a bare `*` are all
understood, since caches and proxies send all three.

What you cannot control:

- **A hard reload** (ctrl+shift+R) bypasses the cache by design. Nothing in a
  header changes that. It will at least come back as a cheap 304.
- **Eviction.** Browsers throw cached things away under storage pressure
  whatever the headers said. Caching is a strong hint, never a guarantee.

If you want to go further:

- **Put a CDN in front.** Browser caching only helps the same visitor twice.
  A CDN caches once for everyone, which is what actually protects the API
  budget when a page is seen by many people. The headers above already say
  everything a CDN needs.
- **A service worker** is the only way to stop the network being touched even
  on a hard reload, and it works offline. Worth it for an app shell, overkill
  for a handful of images.
- **Preload** what you know is coming, with
  `<link rel="preload" as="image" href="...">`, so the fetch starts earlier
  rather than happening less.

## Deleting an image

```bash
curl -X DELETE -H "x-upload-secret: some-secret" localhost:3000/images/6f1d5f4e-....jpg
```

Answers `204` with no body, or `404` if nothing is stored under that name.

Unlike reads, this one needs the secret whenever `UPLOAD_SECRET` is set.
Reading is open, so without a guard here anyone holding a URL could throw away
what it points at.

Under the hood the backend looks up the blob sha first, since the Contents API
will not delete a file without being told exactly which version it means. That
costs one extra API call per delete.

Worth being clear about: this removes the file and writes a commit saying so,
but the commit that added it is still in the history, so anyone with access to
the repo can dig the bytes back out. Treat it as unpublishing, not as
shredding.

## Checking the api budget

```bash
curl -H "x-upload-secret: some-secret" localhost:3000/usage
```

```json
{
  "limit": 5000,
  "used": 37,
  "remaining": 4963,
  "percent_used": 0.74,
  "percent_remaining": 99.26,
  "resets_at": 1786570321,
  "resets_at_utc": "Wednesday, August 12th, 2026 9:32:01pm UTC",
  "resets_in_seconds": 2143,
  "resets_in": "35m 43s",
  "calls_this_window": 12,
  "calls_since_boot": 12,
  "resource": "core",
  "last_call_at": 1786568178,
  "last_call_ago": "4m 12s",
  "calls_total": 431
}
```

The budget numbers follow the `x-ratelimit-*` headers that ride along on every
reply, kept in step with all of them rather than copied from the last one, for
the reason below. `calls_this_window` and `calls_since_boot` are ours: how many
calls this process has made in the hour GitHub is currently counting, and how
many since it started.

`calls_total` only appears when `USAGE_FILE` is set, and counts across every
run rather than this one. `resource` names which of GitHub's budgets the
figures describe, `core` for everything this service does. `last_call_at` and
`last_call_ago` say when GitHub last told us any of this, since between calls
the figures are simply the last thing it said.

The reset is given several ways on purpose. `resets_at` is the raw epoch second
GitHub sent, which is what a script wants, and `resets_at_utc` is the same
moment spelled out for a person reading the endpoint by eye. `resets_in_seconds`
and `resets_in` are the gap from now, worked out fresh on every call.

### Why the reset used to jump about

GitHub hands the same token more than one budget. Uploads and reads come back
from separate counters, each with its own full 5000 and its own hour that
starts the first time you touch it. Both call themselves `core`, so the only
thing in the reply that tells them apart is the reset time.

That is why the reset once flipped between two values depending on whether the
last call was an upload or a fetch. It was two counters answering in turn, not
the hour rolling over, and `used` and `remaining` were flipping with it.

One figure is kept and every reply moves it on. Our own call adds one to it
whichever counter answered, and anything a counter reports beyond that is
spending we did not do, so the larger of the two is taken. A counter that
happens to be behind never drags the count back, and never has to catch up
before it tells us anything. The reset quoted is the earliest hand back we know
of, which keeps it still while the counters take turns.

So `used` is not any one counter's figure, it is what has gone from the
allowance as far as we can tell, which is why `remaining` is worked out from it
rather than taken off the wire. It leans pessimistic, in that two counters each
holding their own 5000 are counted as one, and being told there is less room
than there really is beats the other way round.

It still follows GitHub rather than counting alone. If the same token is in use
anywhere else, from your laptop or another instance, its spending arrives as a
jump larger than one and is taken as it comes. `calls_this_window` stays purely
ours, so the gap between the two is everything spending the token that is not
this service.

### Keeping the count across restarts

The count is written to `./usage.json` by default, so it survives a restart
rather than starting over. Point `USAGE_FILE` somewhere else to move it, or
set it to nothing at all to keep the count in memory only:

```bash
USAGE_FILE=/var/lib/image-upload-proxy/usage.json   # somewhere else
USAGE_FILE=                                          # memory only, no file
```

```json
{
  "calls_total": 431,
  "calls_this_window": 12,
  "last_call_at": 1786568178,
  "window": {
    "resets_at": 1786570321,
    "limit": 5000,
    "used": 37,
    "resource": "core"
  }
}
```

The file is written to a temporary name and renamed over the old one, so a
crash halfway through cannot leave half a file behind, and a file that is
missing or unreadable is reported at boot rather than stopping it.

GitHub's own figures are in there, but only as the last thing it said, with
`last_call_at` saying when. Once the window they belong to has passed they are
ignored outright, and the first look at `/usage` asks GitHub again, which
costs nothing. The file is a memory of what we were told, never the authority
on what is left.

### Seeing the reset on your own clock

GitHub reports the reset in UTC, which is almost never the clock you are
looking at. Set `TIMEZONE` to an IANA zone name and `/usage` carries a
`resets_at_local` line alongside the UTC one:

```bash
TIMEZONE=America/Belize
TIMEZONE=Asia/Kolkata
TIMEZONE=Europe/Madrid
```

```json
"resets_at_utc": "Wednesday, August 12th, 2026 9:32:01pm UTC",
"resets_at_local": "Wednesday, August 12th, 2026 3:32:01pm CST"
```

The UTC line never goes away, since that is what GitHub actually said and it
is the one that matches the header on the wire.

The zone carries its own summer time, so `America/New_York` reads as EDT in
August and EST in February with nothing to change in between. A wrong zone
name stops the process on boot rather than quietly showing the wrong hour.

Zones come from the IANA database, which is compiled into the binary through
`jiff`, so this works the same in a slim container with no tzdata installed.

Both are worth having. GitHub's numbers are the truth, because the same token
may be spending the budget from your laptop or another instance and only
GitHub sees all of it. Ours tell you how much of that spending is this service,
which is what you actually control.

The endpoint sits behind the upload secret. It says nothing about the images,
but it does say how much of the account's budget is left.

### How the limit really works

A token authenticating as a user gets **5000 requests an hour**, per counter,
and as above there is more than one counter. The window does not run to the top
of the clock hour. It opens on your first request and closes an hour later, and
GitHub tells you exactly when in `x-ratelimit-reset`, a UTC epoch second. That
is the moment the count goes back to zero, and it is what this service follows
rather than a timer of its own. Once that moment passes the figures are dropped
rather than kept as a shortage that no longer exists, and the next look at
`/usage` fills them back in.

Reading the budget is free. `GET /rate_limit` is the one endpoint GitHub does
not charge against the primary limit, so when nothing has happened lately and
the numbers have gone cold, `/usage` refreshes from it without spending
anything.

What each request costs you:

| Call | GitHub API calls |
| --- | --- |
| `GET /images/{name}`, uncached | 1 |
| `GET /images/{name}`, revalidating with `If-None-Match` | 0 |
| `POST /upload` | 1 |
| `DELETE /images/{name}` | 2 |
| `GET /usage` | 0 |

Reads are the ones that add up, which is what `CACHE_MAX_AGE` is there to
blunt.

### Responses

| Status | When |
| --- | --- |
| 201 | Stored, the body carries the URL |
| 200 | The image, on a read |
| 204 | Deleted |
| 400 | No file field, or the type is not one `ALLOWED_TYPES` names |
| 401 | `UPLOAD_SECRET` is set and the `x-upload-secret` header is missing or wrong |
| 304 | The caller already holds this image, from `If-None-Match` |
| 404 | Nothing is stored under that name, or the name is not one we hand out |
| 501 | The backend keeps no api budget to report on `/usage` |
| 413 | The file is over `MAX_UPLOAD_MB` |
| 502 | The storage backend refused the request, for example a bad token |
| 500 | The storage backend is misconfigured |

Failures come back as `{ "error": "..." }`. The real upstream message is
written to the logs instead of the response, since it can name the repo or the
token scope.

## Testing with Bruno

The `bruno/` folder is a ready made collection. Open Bruno, choose Open
Collection, point it at that folder, and pick the **Local** environment from
the dropdown in the top right.

The environment holds `baseUrl` and `uploadSecret`. Set `uploadSecret` to
whatever you put in `UPLOAD_SECRET`, or leave it alone if you are running
without auth.

| Request | Expects |
| --- | --- |
| Upload image | 201, and saves the URL as `{{lastUploadUrl}}` |
| Fetch uploaded image | 200, opens the URL the upload returned |
| Delete image | 204, removes what the first request stored |
| Api usage | 200, where the GitHub budget stands |

They run as a sequence, upload then read then delete, so run them in order. The
upload saves both the URL and the bare filename, which is what the delete
needs.

`Upload image` sends `files/sample.png` relative to the collection folder, so
put whatever image you want to test with there under that name.

Run the lot from the terminal with the Bruno CLI:

```bash
cd bruno
npx @usebruno/cli run --env Local
```

A 1x1 png is enough to exercise the whole path:

```bash
mkdir -p bruno/files
base64 -d > bruno/files/sample.png <<'EOF'
iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==
EOF
```

## Logging

Logs go through `tracing`. `LOG_FORMAT=pretty` is the default and gives the
readable multi line console output, `LOG_FORMAT=json` gives one flat object per
line for a log shipper to pick up:

```json
{"timestamp":"2026-08-12T14:37:49.344913Z","level":"WARN","message":"upload rejected","status":400,"reason":"only png, jpg, jpeg, gif, and webp images are accepted","target":"image_upload_proxy::handlers","span":{"method":"POST","path":"/upload","name":"request"}}
```

Every request opens a span holding the method and the path, and closes with a
line carrying the status and the latency. On top of that you get a line when an
upload is stored, when an image is served, when a request is rejected and why,
and when the storage backend fails with the full upstream message attached.

What never reaches the logs: the file bytes, request and response headers, and
the filename the caller chose. Only the size, the extension, and the declared
content type are recorded, so a stray upload secret or an image cannot end up
sitting in your log store.

Use `RUST_LOG` for anything finer, for example `RUST_LOG=image_upload_proxy=debug`
to also see the repo path each upload is written to. The default filter is
`image_upload_proxy=info,tower_http=info`.

## Auth

Setting `UPLOAD_SECRET` mounts a middleware that requires the same value in an
`x-upload-secret` header. Leave the var unset and uploading is open, which the
service warns about on boot. This is worth turning on for anything reachable
from the internet, since every accepted upload writes a commit to your repo.

The middleware guards `POST /upload` and `DELETE /images/{name}`, the two calls
that change something. Reads stay open on purpose, since a secret on
`GET /images/{name}` would mean the URL never renders in a browser or an `img`
tag. Add the same `route_layer` to the `get` in `main.rs` if you want reads
locked down too.

## Adding another storage backend

Every backend implements one trait, `ImageRepository` in `src/repository/mod.rs`,
with a `store` that takes the raw bytes plus an extension and returns the URL,
a `fetch` that reads one back, and a `delete` that removes it. To add a backend
such as Cloudflare R2, drop a new file next to
`src/repository/github.rs`, give it a struct holding whatever config it needs,
implement those three on it, and report problems with the shared `RepoError`
variants so the existing status code mapping keeps working. There is a fourth,
`usage`, which reports on an api budget and defaults to saying nothing, so a
backend without one can leave it alone. The last step is to
build that struct instead of `GitHubRepository` in `main`, which is the only
place the concrete type is named. The handler depends on
`Arc<dyn ImageRepository + Send + Sync>` and does not change at all.

## Layout

```
src/
  main.rs                 wiring, routes, body limit, and request tracing
  config.rs               env vars, read once at boot
  logging.rs              the tracing subscriber, json or pretty
  auth.rs                 the optional shared secret middleware
  handlers.rs             upload, serve, delete, and usage
  usage.rs                what GitHub last said about the rate limit
  repository/
    mod.rs                the ImageRepository trait and RepoError
    github.rs             the GitHub Contents API backend
bruno/                    the api collection, see Testing with Bruno
```
