# Layout

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
bruno/                    the api collection, see docs/bruno.md
```
