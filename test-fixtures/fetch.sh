#!/bin/sh
# Fetch test fixture repos for the parser test harness.
# Run from the test-fixtures/ directory.
set -e

cd "$(dirname "$0")"

clone_if_missing() {
  local dir="$1" tag="$2" url="$3"
  if [ -d "$dir" ]; then
    echo "SKIP $dir (already exists)"
  else
    echo "CLONE $dir ..."
    git clone --depth 1 --branch "$tag" "$url" "$dir"
  fi
}

clone_if_missing click-8.3.1      8.3.1                https://github.com/pallets/click.git
clone_if_missing bat-0.26.1       v0.26.1              https://github.com/sharkdp/bat.git
clone_if_missing axios-1.9.0      v1.9.0               https://github.com/axios/axios.git
clone_if_missing cobra-1.10.2     v1.10.2              https://github.com/spf13/cobra.git
clone_if_missing sidekiq-8.1.1    v8.1.1               https://github.com/sidekiq/sidekiq.git
clone_if_missing gson-2.12.1      gson-parent-2.12.1   https://github.com/google/gson.git
clone_if_missing exposed-0.61.0   0.61.0               https://github.com/JetBrains/Exposed.git
clone_if_missing hono-4.12.8      v4.12.8              https://github.com/honojs/hono.git

echo ""
echo "Done. Run the test harness with:"
echo "  cargo run -p remembrall-test-harness -- --project test-fixtures/<name> --ground-truth test-fixtures/<name>/ground-truth.toml"
