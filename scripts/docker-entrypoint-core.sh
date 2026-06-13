#!/bin/sh
# docker-entrypoint-core.sh — runtime entrypoint for the openhuman-core container.
#
# Problem: Docker named volumes are created owned root:root, even when the image
# has a non-root USER.  The first write openhuman-core makes after the banner
# (init_rpc_token → write_token_file → create_dir_all) hits EACCES and the
# process exits with code 1.
#
# Fix (gosu pattern):
#   1. Start as root so we can chown the mount point(s).
#   2. mkdir -p + chown the workspace directory *before* any application code
#      runs, so the openhuman user owns it regardless of whether Docker created
#      the volume as root.
#   3. exec gosu openhuman to drop privileges and hand off to the binary.
#
# This is idempotent: if the directory already exists with the right ownership
# (image-baked or a re-used volume that was healed on a previous run) the chown
# is a no-op.  No manual "docker volume rm" is required when upgrading from a
# previously broken image.
#
# Requirements: gosu must be installed in the image (see Dockerfile).
# POSIX sh — no bashisms.
set -e

OPENHUMAN_USER="openhuman"
OPENHUMAN_UID="$(id -u "${OPENHUMAN_USER}" 2>/dev/null || echo '')"

# The workspace path the core will actually write to.
# Prefer the env var if set; otherwise fall back to the image default.
WORKSPACE_DIR="${OPENHUMAN_WORKSPACE:-/home/openhuman/.openhuman}"
# The home directory (where core.token is written when OPENHUMAN_CORE_TOKEN is
# unset — see src/core/auth.rs default_root_openhuman_dir()).
HOME_OPENHUMAN_DIR="/home/openhuman/.openhuman"

echo "[docker-entrypoint] uid=$(id -u), gid=$(id -g), user=$(id -un 2>/dev/null || echo unknown)"
echo "[docker-entrypoint] chowning workspace dirs for ${OPENHUMAN_USER} (uid=${OPENHUMAN_UID})"
echo "[docker-entrypoint] WORKSPACE_DIR=${WORKSPACE_DIR}"
echo "[docker-entrypoint] HOME_OPENHUMAN_DIR=${HOME_OPENHUMAN_DIR}"

# Ensure workspace dir exists and is owned by the openhuman user.
mkdir -p "${WORKSPACE_DIR}"
chown "${OPENHUMAN_USER}:${OPENHUMAN_USER}" "${WORKSPACE_DIR}"
echo "[docker-entrypoint] chown ${WORKSPACE_DIR} -> ${OPENHUMAN_USER}:${OPENHUMAN_USER} done"

# If WORKSPACE_DIR and HOME_OPENHUMAN_DIR differ, heal the home dir too
# (core.token always lands in $HOME/.openhuman regardless of OPENHUMAN_WORKSPACE).
if [ "${WORKSPACE_DIR}" != "${HOME_OPENHUMAN_DIR}" ]; then
    mkdir -p "${HOME_OPENHUMAN_DIR}"
    chown "${OPENHUMAN_USER}:${OPENHUMAN_USER}" "${HOME_OPENHUMAN_DIR}"
    echo "[docker-entrypoint] chown ${HOME_OPENHUMAN_DIR} -> ${OPENHUMAN_USER}:${OPENHUMAN_USER} done"
fi

echo "[docker-entrypoint] dropping privileges -> exec gosu ${OPENHUMAN_USER} openhuman-core $*"
exec gosu "${OPENHUMAN_USER}" openhuman-core "$@"
