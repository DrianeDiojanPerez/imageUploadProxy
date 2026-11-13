# Checking the api budget

```bash
curl -H "x-upload-secret: some-secret" localhost:3000/usage
```

```json
{
  "limit": 5000,
  "used": 37,
  "remaining": 4963,
  "percent_used": 0.74,
  "percent_remaining": 99.26,
  "resets_at": 1786570321,
  "resets_at_utc": "Wednesday, August 12th, 2026 9:32:01pm UTC",
  "resets_in_seconds": 2143,
  "resets_in": "35m 43s",
  "calls_this_window": 12,
  "calls_since_boot": 12,
  "resource": "core",
  "last_call_at": 1786568178,
  "last_call_ago": "4m 12s",
  "calls_total": 431
}
```

The budget numbers follow the `x-ratelimit-*` headers that ride along on every
reply, kept in step with all of them rather than copied from the last one, for
the reason below. `calls_this_window` and `calls_since_boot` are ours: how many
calls this process has made in the hour GitHub is currently counting, and how
many since it started.

`calls_total` only appears when `USAGE_FILE` is set, and counts across every
run rather than this one. `resource` names which of GitHub's budgets the
figures describe, `core` for everything this service does. `last_call_at` and
`last_call_ago` say when GitHub last told us any of this, since between calls
the figures are simply the last thing it said.

The reset is given several ways on purpose. `resets_at` is the raw epoch second
GitHub sent, which is what a script wants, and `resets_at_utc` is the same
moment spelled out for a person reading the endpoint by eye. `resets_in_seconds`
and `resets_in` are the gap from now, worked out fresh on every call.

## Why the reset used to jump about

GitHub hands the same token more than one budget. Uploads and reads come back
from separate counters, each with its own full 5000 and its own hour that
starts the first time you touch it. Both call themselves `core`, so the only
thing in the reply that tells them apart is the reset time.

That is why the reset once flipped between two values depending on whether the
last call was an upload or a fetch. It was two counters answering in turn, not
the hour rolling over, and `used` and `remaining` were flipping with it.

One figure is kept and every reply moves it on. Our own call adds one to it
whichever counter answered, and anything a counter reports beyond that is
spending we did not do, so the larger of the two is taken. A counter that
happens to be behind never drags the count back, and never has to catch up
before it tells us anything. The reset quoted is the earliest hand back we know
of, which keeps it still while the counters take turns.

So `used` is not any one counter's figure, it is what has gone from the
allowance as far as we can tell, which is why `remaining` is worked out from it
rather than taken off the wire. It leans pessimistic, in that two counters each
holding their own 5000 are counted as one, and being told there is less room
than there really is beats the other way round.

It still follows GitHub rather than counting alone. If the same token is in use
anywhere else, from your laptop or another instance, its spending arrives as a
jump larger than one and is taken as it comes. `calls_this_window` stays purely
ours, so the gap between the two is everything spending the token that is not
this service.

## Keeping the count across restarts

The count is written to `./usage.json` by default, so it survives a restart
rather than starting over. Point `USAGE_FILE` somewhere else to move it, or
set it to nothing at all to keep the count in memory only:

```bash
USAGE_FILE=/var/lib/image-upload-proxy/usage.json   # somewhere else
USAGE_FILE=                                          # memory only, no file
```

```json
{
  "calls_total": 431,
  "calls_this_window": 12,
  "last_call_at": 1786568178,
  "window": {
    "resets_at": 1786570321,
    "limit": 5000,
    "used": 37,
    "resource": "core"
  }
}
```

The file is written to a temporary name and renamed over the old one, so a
crash halfway through cannot leave half a file behind, and a file that is
missing or unreadable is reported at boot rather than stopping it.

GitHub's own figures are in there, but only as the last thing it said, with
`last_call_at` saying when. Once the window they belong to has passed they are
ignored outright, and the first look at `/usage` asks GitHub again, which
costs nothing. The file is a memory of what we were told, never the authority
on what is left.

## Seeing the reset on your own clock

GitHub reports the reset in UTC, which is almost never the clock you are
looking at. Set `TIMEZONE` to an IANA zone name and `/usage` carries a
`resets_at_local` line alongside the UTC one:

```bash
TIMEZONE=America/Belize
TIMEZONE=Asia/Kolkata
TIMEZONE=Europe/Madrid
```

```json
"resets_at_utc": "Wednesday, August 12th, 2026 9:32:01pm UTC",
"resets_at_local": "Wednesday, August 12th, 2026 3:32:01pm CST"
```

The UTC line never goes away, since that is what GitHub actually said and it
is the one that matches the header on the wire.

The zone carries its own summer time, so `America/New_York` reads as EDT in
August and EST in February with nothing to change in between. A wrong zone
name stops the process on boot rather than quietly showing the wrong hour.

Zones come from the IANA database, which is compiled into the binary through
`jiff`, so this works the same in a slim container with no tzdata installed.

Both are worth having. GitHub's numbers are the truth, because the same token
may be spending the budget from your laptop or another instance and only
GitHub sees all of it. Ours tell you how much of that spending is this service,
which is what you actually control.

The endpoint sits behind the upload secret. It says nothing about the images,
but it does say how much of the account's budget is left.

## How the limit really works

A token authenticating as a user gets **5000 requests an hour**, per counter,
and as above there is more than one counter. The window does not run to the top
of the clock hour. It opens on your first request and closes an hour later, and
GitHub tells you exactly when in `x-ratelimit-reset`, a UTC epoch second. That
is the moment the count goes back to zero, and it is what this service follows
rather than a timer of its own. Once that moment passes the figures are dropped
rather than kept as a shortage that no longer exists, and the next look at
`/usage` fills them back in.

Reading the budget is free. `GET /rate_limit` is the one endpoint GitHub does
not charge against the primary limit, so when nothing has happened lately and
the numbers have gone cold, `/usage` refreshes from it without spending
anything.

What each request costs you:

| Call | GitHub API calls |
| --- | --- |
| `GET /images/{name}`, uncached | 1 |
| `GET /images/{name}`, revalidating with `If-None-Match` | 0 |
| `POST /upload` | 1 |
| `DELETE /images/{name}` | 2 |
| `GET /usage` | 0 |

Reads are the ones that add up, which is what `CACHE_MAX_AGE` is there to
blunt.
