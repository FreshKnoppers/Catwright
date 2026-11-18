#!/bin/sh
# Render the README screenshots against a stand-in for vibe-acp that
# replays prepared demo sessions (no Vibe account, no network, nothing
# outside a temp folder is touched). Usage: tools/screenshots/render.sh [out-dir]
# Needs python3 and git, no display: the app renders with --headless
# (CPU rasterizer, app/src/softrender.rs), so this also runs in sandboxes
# without X11, Wayland or a GPU.
set -e
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo="$here/../.."
out="${1:-$repo/docs/screenshots}"
mkdir -p "$out"
out=$(CDPATH= cd -- "$out" && pwd)
D=$(mktemp -d)
eval "$(python3 -I "$here/build.py" "$D")"
mkdir -p "$D/bin" && cp "$here/vibe-acp" "$D/bin/vibe-acp" && chmod +x "$D/bin/vibe-acp"
(cd "$repo/app" && cargo build --release)
bin="$repo/app/target/release/catwright"
shot() {
    name=$1; cfg=$2; shift 2
    env HOME="$D/home" XDG_CONFIG_HOME="$D/cfg-$cfg" VIBE_HOME="$D/home/.vibe" \
        PATH="$D/bin:$PATH" DEMO_SCENARIO="$D/scenario.json" \
        "$bin" "$@" --screenshot "$out/$name.png" --frames 240 --lang en --headless
}
shot start start --project "$WS"
shot session session --project "$WS" --session "$S_STATS"
shot subagents subagents --project "$WS" --session "$S_SUB"
shot dashboard dashboard --project "$HC" --session "$S_DASH"
rm -rf "$D"
echo "screenshots in $out"
