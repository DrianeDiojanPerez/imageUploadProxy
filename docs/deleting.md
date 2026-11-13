# Deleting an image

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
