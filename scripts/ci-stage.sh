#!/usr/bin/env bash
# Keep CI stage selection in Make; retain output and timing even on failure.
set -euo pipefail
if [[ $# != 1 ]]; then
    echo 'Usage: scripts/ci-stage.sh CHECK_STAGE' >&2
    exit 2
fi
stage=$1
case "$stage" in
    check-static|check-clippy|check-compat|check-default-remaining|check-proving|check-lifecycle|check-codec) ;;
    *) echo "Unknown CI check stage: $stage" >&2; exit 2 ;;
esac
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
mkdir -p target/ci-checks
started=$SECONDS
set +e
make "$stage" 2>&1 | tee "target/ci-checks/$stage.log"
pipeline_status=("${PIPESTATUS[@]}")
set -e
elapsed=$((SECONDS - started))
status=${pipeline_status[0]}
if [[ $status == 0 ]]; then
    status=${pipeline_status[1]}
fi
printf 'elapsed_seconds=%s\nexit_status=%s\n' "$elapsed" "$status" \
    > "target/ci-checks/$stage.status"
if [[ -n ${GITHUB_STEP_SUMMARY:-} ]]; then
    printf '### %s\n\nElapsed: %s seconds; exit status: %s.\n\n' \
        "$stage" "$elapsed" "$status" >> "$GITHUB_STEP_SUMMARY"
fi
exit "$status"
