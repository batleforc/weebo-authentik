#!/bin/sh
# Shared by e2e.yml's steps: each `run:` block in a GitHub Actions job is
# its own separate shell, so a function defined in one step isn't visible
# to the next — source this file at the top of each step that needs to
# poll something until ready or fail with diagnostics.

# Calls $3 (a shell function name, no args) up to $2 times, sleeping 5s
# between attempts, until it returns 0; on exhaustion, prints ::error:: for
# $1 and calls $4 (another function name) for diagnostics before returning
# 1. The single retry/backoff/error-reporting shape shared by every "poll
# until ready" check in this workflow: the kubectl CRD-readiness check
# below (`wait_ready`), and the docker-compose Authentik-server health
# check in e2e.yml's "Start a real, throwaway Authentik instance" step.
retry_until() {
    desc="$1"
    attempts="$2"
    check="$3"
    diagnostics="$4"
    echo "waiting for ${desc}..."
    for _ in $(seq 1 "$attempts"); do
        if "$check"; then
            return 0
        fi
        sleep 5
    done
    echo "::error::${desc} never became ready within the timeout"
    "$diagnostics"
    return 1
}

wait_ready() {
    local kind="$1" name="$2" ns_args="$3"
    _wait_ready_check() {
        ready=$(mise exec -- kubectl get "$kind" "$name" $ns_args \
            -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' 2>/dev/null || true)
        id=$(mise exec -- kubectl get "$kind" "$name" $ns_args \
            -o jsonpath='{.status.authentikId}' 2>/dev/null || true)
        if [ "$ready" = "True" ] && [ -n "$id" ]; then
            echo "${kind}/${name} synced with authentikId=$id"
            return 0
        fi
        return 1
    }
    _wait_ready_diagnostics() {
        mise exec -- kubectl get "$kind" "$name" $ns_args -o yaml
    }
    retry_until "${kind}/${name} to sync" 30 _wait_ready_check _wait_ready_diagnostics
}
