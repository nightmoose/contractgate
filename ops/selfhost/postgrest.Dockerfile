# PostgREST's published image has no shell. Copy the static binary onto
# Alpine so the self-host entrypoint can load generated secrets.
FROM postgrest/postgrest:v12.2.12 AS bin
FROM alpine:3.20
COPY --from=bin /bin/postgrest /usr/local/bin/postgrest
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/postgrest"]
