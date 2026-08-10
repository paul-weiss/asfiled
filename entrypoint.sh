#!/bin/sh
# With LITESTREAM_REPLICA_URL set (e.g. s3://asfiled-data/users), the users
# DB is restored from the replica on boot and replicated continuously —
# survives redeploys on hosts with no persistent disk. Without it, run the
# server directly (local development).
set -e

if [ -n "$LITESTREAM_REPLICA_URL" ]; then
    litestream restore -if-db-not-exists -if-replica-exists -o "$ASFILED_USERS_DB" "$LITESTREAM_REPLICA_URL"
    exec litestream replicate -exec "/app/server" "$ASFILED_USERS_DB" "$LITESTREAM_REPLICA_URL"
else
    exec /app/server
fi
