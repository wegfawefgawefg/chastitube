#!/bin/sh
# Sets chastitube up for this user: builds it, link handler, login autostart,
# server now. Run again after changing the code or moving the folder.
# Undo: ./install.sh --uninstall
# Topic tags (optional, off unless asked for): ./install.sh --tags builds the
# tagger and downloads its model (1.7 GB); ./install.sh --tags-off turns them off.
# http://chastitube instead of http://localhost:8790: ./install.sh --name (asks for sudo).
set -e
DIR=$(cd "$(dirname "$0")" && pwd)
APPS="$HOME/.local/share/applications"
AUTO="$HOME/.config/autostart"
BIN="$DIR/bin/chastitube"
PORT=8790
DATA="${CHASTITUBE_DATA_DIR:-$HOME/.local/share/chastitube}"
MODEL_URL=https://huggingface.co/desplega/laya-onnx/resolve/main/typed-decisions/fp32

if [ "$1" = "--uninstall" ]; then
    xdg-mime default firefox.desktop x-scheme-handler/http x-scheme-handler/https
    rm -f "$APPS/chastitube-open.desktop" "$AUTO/chastitube.desktop" "$HOME/.local/bin/chube" "$HOME/.local/bin/chastitube"
    echo "Links go straight to Firefox again. Stop the server with: kill \$(ss -ltnp 'sport = :$PORT' | grep -oP 'pid=\\K\\d+')"
    exit 0
fi

if [ "$1" = "--tags-off" ]; then
    rm -f "$DIR/bin/chastitube-tagger"
    echo "Topic tags are off. Tags so far stay in $DATA; delete $DATA/laya to free the model's space."
    exit 0
fi

(cd "$DIR" && cargo build --release --quiet)
mkdir -p "$DIR/bin" "$APPS" "$AUTO" "$HOME/Videos/chastitube"
# copy then rename, so a running server's binary is never written over
cp "$DIR/target/release/chastitube" "$BIN.new" && mv "$BIN.new" "$BIN"

# the tagger: built when asked for, and kept up to date once it's there
if [ "$1" = "--tags" ] || [ -f "$DIR/bin/chastitube-tagger" ]; then
    (cd "$DIR/tagger" && cargo build --release --quiet)
    cp "$DIR/tagger/target/release/chastitube-tagger" "$DIR/bin/chastitube-tagger.new"
    mv "$DIR/bin/chastitube-tagger.new" "$DIR/bin/chastitube-tagger"
fi
if [ "$1" = "--tags" ] && [ ! -f "$DATA/laya/encoder.onnx" ]; then
    echo "Downloading the tagging model (Laya, 1.7 GB) into $DATA/laya"
    mkdir -p "$DATA/laya.part"
    for f in tokenizer.json rl_agent_config.json head.onnx encoder.onnx; do
        curl -L --fail --progress-bar -C - -o "$DATA/laya.part/$f" "$MODEL_URL/$f"
    done
    mv "$DATA/laya.part" "$DATA/laya"
fi

# `chube` or `chastitube` in dmenu (or a terminal) opens the page
mkdir -p "$HOME/.local/bin"
ln -sf "$BIN" "$HOME/.local/bin/chube"
ln -sf "$BIN" "$HOME/.local/bin/chastitube"

cat > "$APPS/chastitube-open.desktop" <<EOD
[Desktop Entry]
Type=Application
Name=chastitube link handler
Comment=YouTube links play without the feed; other links open in Firefox
Exec=$BIN open %U
MimeType=x-scheme-handler/http;x-scheme-handler/https;
NoDisplay=true
EOD

cat > "$AUTO/chastitube.desktop" <<EOD
[Desktop Entry]
Type=Application
Name=chastitube
Exec=$BIN serve
NoDisplay=true
EOD

cp -n "$HOME/.config/mimeapps.list" "$HOME/.config/mimeapps.list.before-chastitube" 2>/dev/null || true
update-desktop-database "$APPS" 2>/dev/null || true
xdg-mime default chastitube-open.desktop x-scheme-handler/http x-scheme-handler/https

# http://chastitube: the name on chastitube's own loopback address (so port 80
# there can't clash with anything on localhost), low ports allowed for normal
# programs, and Firefox told that the bare word `chastitube` is a site
if [ "$1" = "--name" ]; then
    echo "Setting up http://chastitube (sudo for /etc/hosts, /etc/sysctl.d and Firefox's policy file)"
    grep -q "[[:space:]]chastitube\([[:space:]]\|\$\)" /etc/hosts || echo "127.0.0.73 chastitube" | sudo tee -a /etc/hosts >/dev/null
    echo "net.ipv4.ip_unprivileged_port_start = 80" | sudo tee /etc/sysctl.d/60-chastitube.conf >/dev/null
    sudo sysctl -q -w net.ipv4.ip_unprivileged_port_start=80
    sudo "$BIN" firefox-policy
fi

# restart the server on the new build
OLD=$(ss -ltnp "sport = :$PORT" | grep -oP 'pid=\K\d+' || true)
[ -n "$OLD" ] && kill $OLD && sleep 0.5
DISPLAY=${DISPLAY:-:0} setsid "$BIN" serve >/dev/null 2>&1 < /dev/null &
sleep 1
if getent hosts chastitube >/dev/null && ss -ltn "sport = :80" | grep -q 127.0.0.73; then
    echo "chastitube is on http://chastitube (and http://localhost:$PORT); type chastitube in Firefox, or chube in dmenu"
else
    echo "chastitube is on http://localhost:$PORT; chube in dmenu opens it. For http://chastitube run ./install.sh --name"
fi
