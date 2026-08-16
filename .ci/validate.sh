#!/usr/bin/env bash
# Container entrypoint for `compose.yaml`'s validation services. Resolves
# the envtest/bindgen toolchain paths for this image (they differ from the
# Che-host defaults baked into Taskfile.yaml's env: block), installs the
# mise-declared tools, then runs whatever `task` targets were passed as
# arguments. Defaults to the CI code-validation set: lint + unit + envtest
# integration tests.
set -euo pipefail

cd /workspace

# mise refuses to load a config it hasn't been told to trust.
mise trust --quiet /workspace/mise.toml

# LIBCLANG_PATH / BINDGEN_EXTRA_CLANG_ARGS: Taskfile.yaml's defaults point
# at the Che host's llvm + gcc-15 layout. This image is debian bookworm
# (gcc 12, distro libclang), so resolve both from what's actually on disk
# and export them — the Taskfile honours a pre-set value over its default.
libclang_so="$(find /usr/lib /usr/lib64 -iname 'libclang.so*' 2>/dev/null | head -n1 || true)"
if [ -z "${libclang_so}" ]; then
    echo "error: libclang.so not found — envtest's bindgen build cannot run" >&2
    exit 1
fi
export LIBCLANG_PATH="$(dirname "${libclang_so}")"

gcc_inc="$(find /usr/lib/gcc/*/*/include -maxdepth 0 -type d 2>/dev/null | head -n1 || true)"
export BINDGEN_EXTRA_CLANG_ARGS="${gcc_inc:+-I${gcc_inc} }-I/usr/include"

echo "== weebo-authentik validation =="
echo "  LIBCLANG_PATH=${LIBCLANG_PATH}"
echo "  BINDGEN_EXTRA_CLANG_ARGS=${BINDGEN_EXTRA_CLANG_ARGS}"

# Install the toolchains mise.toml declares (rust, go, task, etcd,
# kube-apiserver, helm, ...). Cached in the /mise volume between runs.
mise install

# Default to the code-validation targets CI runs; allow overriding via the
# service command (e.g. `lint:helm`, `test`, `test:integration`).
targets=("$@")
if [ "${#targets[@]}" -eq 0 ]; then
    targets=(lint test test:integration)
fi

for target in "${targets[@]}"; do
    echo "== task ${target} =="
    mise exec -- task "${target}"
done

echo "== validation complete =="
