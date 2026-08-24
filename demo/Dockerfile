# The public demo runs the release tarball itself — the same artifact people
# download from GitHub — so the demo cannot drift from what is shipped.
# Rebuilding on a new release is: drop the new tarball in build/, redeploy.
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# install_root() finds the assets from the binary at <root>/target/release/,
# which is the layout inside the tarball, so nothing needs pointing at anything.
ARG TARBALL=kotodex-demo-linux-x86_64.tar.gz
COPY build/${TARBALL} /tmp/kotodex.tar.gz
RUN tar -xzf /tmp/kotodex.tar.gz --strip-components=1 -C /app \
 && rm /tmp/kotodex.tar.gz

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

EXPOSE 3299
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
