# Reading an image back

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

## Caching

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
