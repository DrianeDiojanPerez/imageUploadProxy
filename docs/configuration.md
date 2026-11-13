# Configuration

Every setting is an env var, read from `.env` or the shell.

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
| `UPLOAD_SECRET` | unset | Guards writing, see [Auth](auth.md) |
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
