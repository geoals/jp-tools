# The public demo

`demo.kotodex.com` — the dashboard, served from a frozen seed, with every
request that is not a GET refused (`KOTODEX_DEMO=1`).

It runs **the release tarball itself**, so the demo cannot drift from what
people download. A new release is deployed by dropping its tarball in `build/`
and rebuilding the image; nothing here is built from source.

## The seed

`scripts/make-demo-data.py` turns a copy of the live databases into the seed.
Titles, covers, vocabulary and every timestamp and count are the reader's own,
because a list of what someone read is a bookshelf. The hooked lines and the
books are replaced, because those are the works themselves and the demo serves
them over public GETs — `/api/lines/before` pages back through the whole
stream.

The replacement prose comes from `scripts/aozora_corpus.py`: public-domain
works from Aozora Bunko, fetched to a cache in `target/`. It is sized to the
number of characters actually read and stops there, mid-work if that is where
the total lands. Real prose rather than a sentence bank because the kanji grid
and "new kanji per day" count distinct characters — a small bank gives every
kanji the same count on one day, which is not what a reading history looks
like. Each work walks its own stretch of the corpus in timestamp order, so new
characters arrive at the pace a book introduces them.

`--today` (default 2026-08-22) is the day the container's `KOTODEX_DEMO_TODAY`
pins the clock to. Everything after it is dropped, so Today always has reading
under it and nothing sits in the dashboard's future. The two have to agree.

The seed is deliberately not regenerated per release. It only needs redoing
when a new panel reads data it has none of.

## Where it runs

TrueNAS, as a Dockge stack in
`/mnt/ssd-pool/apps/dockge/stacks/kotodex/`:

```
build/kotodex-<version>-linux-x86_64.tar.gz   the release, unpacked into the image
seed/                                         knowledge.db, kotodex.db, covers/
```

The seed is mounted read-only and copied to scratch inside the container at
start, so the release's migrations have somewhere to run and a restart puts the
data back.

Caddy fronts it. Both names are proxied through Cloudflare, so the certificates
come from the DNS challenge — an HTTP one is answered by the edge:

```
kotodex.com       root * /srv/kotodex, file_server   (site/index.html)
demo.kotodex.com  reverse_proxy 192.168.0.8:3299
```

## Deploying a new release

```
scripts/build-release.sh <version>       # in rust:1-bookworm, for the glibc floor
scp target/release-artifact/*.tar.gz  <nas>:.../kotodex/build/
ssh <nas> 'cd .../kotodex && docker compose build && docker compose up -d'
```

The tarball must be built in `rust:1-bookworm` — a build on a host with a newer
glibc will not start on the bookworm base image.
