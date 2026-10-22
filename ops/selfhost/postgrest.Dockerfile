# PostgREST's published image has no shell. Copy the static binary onto
# Alpine so the self-host entrypoint can load generated secrets.
FROM postgrest/postgrest:v12.2.12 AS bin
FROM alpine:3.20
COPY --from=bin /bin/postgrest /usr/local/bin/postgrest
# Non-root. Compose overrides the entrypoint to /bin/sh with-secrets.sh,
# which only needs to read the mode-0644 secrets file and exec this binary.
# The server listens on 3000, which does not require root.
RUN adduser -D -H -u 1001 postgrest \
    && chmod 0755 /usr/local/bin/postgrest
USER postgrest
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/postgrest"]
