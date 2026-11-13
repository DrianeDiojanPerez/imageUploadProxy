# Auth

Setting `UPLOAD_SECRET` mounts a middleware that requires the same value in an
`x-upload-secret` header. Leave the var unset and uploading is open, which the
service warns about on boot. This is worth turning on for anything reachable
from the internet, since every accepted upload writes a commit to your repo.

The middleware guards `POST /upload` and `DELETE /images/{name}`, the two calls
that change something. Reads stay open on purpose, since a secret on
`GET /images/{name}` would mean the URL never renders in a browser or an `img`
tag. Add the same `route_layer` to the `get` in `main.rs` if you want reads
locked down too.
