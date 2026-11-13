# Adding another storage backend

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
