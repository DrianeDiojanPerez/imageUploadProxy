# Responses

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
