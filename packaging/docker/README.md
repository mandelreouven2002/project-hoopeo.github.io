# DEP Engine in a container

Two Dockerfiles, the same image at run time:

- `Dockerfile.binary` packages the prebuilt Linux binaries of a release. This is the image that is published:
  the `release` workflow builds it from the release tarballs and pushes it to GitHub Container Registry as
  `ghcr.io/mandelreouven2002/dep-engine:<version>` and `:latest`, a public package linked to the public
  repository. `make-context.sh` puts the Dockerfile, the binaries and the notices in one folder
  (`dist/docker/dep-engine-<v>-docker-<arch>/`), and in that folder `docker build -t dep-engine:<v> .` builds it.
- `Dockerfile` builds DEP Engine from the source in this repository (multi-stage, about 10 minutes the first
  time). It is for local use; the image it makes is not published.

```sh
docker run -d --name dep-engine -p 5433:5433 -v dep-data:/var/lib/dep-engine \
  -e DEP_ADMIN_PASSWORD=change-me ghcr.io/mandelreouven2002/dep-engine:latest
dep-cli -h 127.0.0.1 -p 5433 -U admin -d operational --sslmode require
```

The image is for evaluation, under the Project Hoopeo Evaluation License; its notices are in
`/usr/share/doc/project-hoopeo`.

Configuration is by environment variable: `DEP_LISTEN`, `DEP_DATA_DIR`, `DEP_ADMIN_PASSWORD`, `DEP_INIT`,
`DEP_TLS_CERT` and `DEP_TLS_KEY` (or `DEP_TLS_SELF_SIGNED=1`, the default in the image, with `DEP_TLS_HOSTS`),
`DEP_TLS_REQUIRED=1`, `DEP_HTTP_LISTEN=0.0.0.0:8433` for the HTTP API (publish port 8433 too),
`DEP_MATCH_LIMIT=1000/minute`, and the resource settings `DEP_MEMORY_LIMIT`, `DEP_CACHE_SIZE`,
`DEP_IDLE_IN_TRANSACTION_TIMEOUT` and `DEP_MAX_CONNECTIONS`. Their defaults follow the container's memory
limit (a quarter of it for queries, an eighth for the data file's cache), not the host's memory.
Keep `/var/lib/dep-engine` on a volume: it holds the database and its master key. To keep the key
apart from the data, mount it elsewhere and set `DEP_KEY_FILE`.

On Railway: deploy this Dockerfile as a service, attach a volume at `/var/lib/dep-engine`, set
`DEP_ADMIN_PASSWORD`, and expose port 5433 through a TCP proxy. Clients connect to the proxy host and
port with `--sslmode require`.
