# The public demo

`demo.kotodex.com` — the dashboard, served from a frozen seed, with every
request that is not a GET refused (`KOTODEX_DEMO=1`).

It runs **the release tarball itself**, so the demo cannot drift from what
people download. A new release is deployed by dropping its tarball in `build/`
and rebuilding the image; nothing here is built from source.

## The seed

`scripts/make-demo-data.py` turns a copy of the live databases into the seed.
No text from outside this repository survives it — titles, lines, books, mined
cards and looked-up words are all replaced, and the dictionary cache is
emptied. What is kept is every timestamp and count, so the pacing and the
streaks are a real reader's.

The seed is deliberately not regenerated per release. It only needs redoing
when a new panel reads data it has none of.

## Where it runs

TrueNAS, as a Dockge stack in
`/mnt/ssd-pool/apps/dockge/stacks/kotodex/`:

```
build/kotodex-<version>-linux-x86_64.tar.gz   the release, unpacked into the image
seed/                                         knowledge.db, read-stats.db, covers/
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
