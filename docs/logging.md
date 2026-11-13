# Logging

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
