#!/bin/sh
# Shared by e2e.yml's "Golden path" steps: each `run:` block in a GitHub
# Actions job is its own separate shell, so a function defined in one step
# isn't visible to the next — source this file at the top of each step that
# needs to poll a CR for Ready=True + an authentikId instead of redefining
# the loop inline.
wait_ready() {
    kind="$1"
    name="$2"
    ns_args="$3"
    echo "waiting for ${kind}/${name} to sync..."
    for _ in $(seq 1 30); do
        ready=$(mise exec -- kubectl get "$kind" "$name" $ns_args \
            -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' 2>/dev/null || true)
        id=$(mise exec -- kubectl get "$kind" "$name" $ns_args \
            -o jsonpath='{.status.authentikId}' 2>/dev/null || true)
        if [ "$ready" = "True" ] && [ -n "$id" ]; then
            echo "${kind}/${name} synced with authentikId=$id"
            return 0
        fi
        sleep 5
    done
    echo "::error::${kind}/${name} never reached Ready=True with an authentikId"
    mise exec -- kubectl get "$kind" "$name" $ns_args -o yaml
    return 1
}
