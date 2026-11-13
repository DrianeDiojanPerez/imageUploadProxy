# Uploading

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
