# Testing with Bruno

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
