#!/bin/bash
# README screenshots from the installed Mihomo-Monitor.app, on fictional demo data.
#
#   app/scripts/readme-screenshots.sh [output folder]      (default: docs/images)
#
# macOS only. The app runs with MIHOMO_MONITOR_HOME pointing at a demo folder, with automatic
# discovery off and one manually added core: scripts/fake-mihomo.py on 127.0.0.1:19090. So it
# never sees your own cores or traffic, and your statistics are untouched. A year of history
# comes from scripts/demo-data.py. Pages are opened by pressing the app's buttons through macOS
# accessibility; when a button cannot be found the script asks you to click it yourself.
# Windows are captured with `screencapture`, which keeps the native frame and shadow.
#
# Needs, for the terminal app this runs in: Accessibility (to press buttons) and Screen Recording
# (to capture another app's window), under System Settings → Privacy & Security.
# Environment: MM_APP (default /Applications/Mihomo-Monitor.app), MM_DEMO (demo folder),
# MANUAL=1 to open every page by hand.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
APP=${MM_APP:-/Applications/Mihomo-Monitor.app}
DEMO=${MM_DEMO:-/Users/Shared/MihomoMonitorDemo}
PORT=19090
OUT=$(mkdir -p "${1:-$REPO/docs/images}" && cd "${1:-$REPO/docs/images}" && pwd)
WORK=$(mktemp -d)
PID=""
CORE=""

say() { printf '\033[1m%s\033[0m\n' "$*"; }
fail() { printf '\033[31m%s\033[0m\n' "$*" >&2; exit 1; }

[ "$(uname)" = Darwin ] || fail "只能在 macOS 上运行。"
[ -d "$APP" ] || fail "找不到 $APP。可以用 MM_APP=/path/to/Mihomo-Monitor.app 指定。"
command -v python3 >/dev/null || fail "需要 python3（xcode-select --install 会一并装上）。"
EXE="$APP/Contents/MacOS/$(/usr/bin/plutil -extract CFBundleExecutable raw -o - "$APP/Contents/Info.plist")"

cleanup() {
  [ -n "$PID" ] && kill "$PID" 2>/dev/null || true
  [ -n "$CORE" ] && kill "$CORE" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# ---- the real Mihomo-Monitor has to be out of the way: only one instance runs at a time ---

if pgrep -f "$EXE" >/dev/null; then
  say "Mihomo-Monitor 正在运行。请从菜单栏图标选择「退出」（截图结束后再打开）。"
  say "只要内核没有重启，退出期间的流量会在重新打开后补回。"
  while pgrep -f "$EXE" >/dev/null; do sleep 1; done
fi
if lsof -nP -iTCP:$PORT -sTCP:LISTEN >/dev/null 2>&1; then
  fail "端口 $PORT 已被占用，模拟内核无法启动。"
fi

# ---- demo data ----------------------------------------------------------------------------

say "准备示例数据：$DEMO"
rm -rf "$DEMO"
mkdir -p "$DEMO"
KEY="manual:http://127.0.0.1:$PORT"
cat > "$DEMO/settings.json" <<JSON
{
  "lightweight": true,
  "autostartInitialized": true,
  "weekStartsSunday": false,
  "autoDiscover": false,
  "manualSources": [{ "name": "家里的路由器", "url": "http://127.0.0.1:$PORT", "secret": "" }]
}
JSON

python3 "$HERE/fake-mihomo.py" --port $PORT >/dev/null 2>&1 &
CORE=$!

# The app creates its database and the source; history is added once both exist.
MIHOMO_MONITOR_HOME="$DEMO" "$EXE" --tray >/dev/null 2>&1 &
PID=$!
for _ in $(seq 1 60); do
  python3 -c "import sqlite3,sys; sys.exit(0 if sqlite3.connect('$DEMO/traffic.db').execute(\"SELECT 1 FROM sources WHERE key='$KEY'\").fetchone() else 1)" 2>/dev/null && break
  sleep 0.5
done
kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
PID=""
python3 "$HERE/demo-data.py" "$DEMO/traffic.db" "$KEY"

# ---- driving the app ----------------------------------------------------------------------

cat > "$WORK/ui.js" <<'JS'
// osascript -l JavaScript ui.js <command> <pid> [arguments]
//   window <pid>                         id of the app's main window, "" until it is on screen
//   front <pid>                          bring the app forward
//   press <pid> <text> [index] [exact]   press the index-th control whose label contains (or equals) text
//   label <pid> <text>                   label of the first control whose label contains text
//   dump <pid>                           every control and its label, to find names when a press fails
ObjC.import("CoreGraphics");
function run(argv) {
  const [cmd, pid, text, index, exact] = argv;
  if (cmd === "window") {
    const list = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, 0))) || [];
    const w = list.find((w) => w.kCGWindowOwnerPID == pid && w.kCGWindowLayer == 0 && w.kCGWindowBounds.Width > 400);
    return w ? String(w.kCGWindowNumber) : "";
  }
  const proc = Application("System Events").processes.whose({ unixId: Number(pid) })[0];
  if (cmd === "front") { proc.frontmost = true; return "ok"; }
  const roles = ["AXButton", "AXDisclosureTriangle", "AXLink", "AXRadioButton", "AXTab"];
  const label = (e) => ["title", "name", "description"].map((k) => { try { return String(e[k]() || ""); } catch (x) { return ""; } }).filter(Boolean).join(" ");
  const controls = proc.windows[0].entireContents().filter((e) => { try { return roles.includes(e.role()); } catch (x) { return false; } });
  if (cmd === "dump") return controls.map((e) => e.role() + "  " + label(e)).join("\n");
  const hits = controls.filter((e) => (exact ? label(e).split(" ").includes(text) : label(e).includes(text)));
  if (cmd === "label") return hits.length ? label(hits[0]) : "";
  const hit = hits[Number(index || 0)];
  if (!hit) return "missing";
  hit.actions.byName("AXPress").perform();
  return "ok";
}
JS
ui() { osascript -l JavaScript "$WORK/ui.js" "$@" 2>/dev/null || true; }

# Presses a control; when it is not there, asks for the click instead. $2 says what to click.
press() {
  local text=$1 hint=$2 index=${3:-0} exact=${4:-}
  if [ "${MANUAL:-0}" != 1 ] && [ "$(ui press "$PID" "$text" "$index" "$exact")" = ok ]; then sleep 1.2; return; fi
  ui front "$PID"
  printf '  请在 Mihomo-Monitor 窗口里%s，然后回到这里按回车… ' "$hint"
  read -r _
}

# Cycles the theme button until it shows the wanted mode (浅色 / 深色 / 跟随系统).
theme() {
  for _ in 1 2 3 4; do
    [ "$(ui label "$PID" "主题：")" = "主题：$1" ] && return
    ui press "$PID" "主题：" >/dev/null; sleep 0.6
  done
  press "主题：" "点右上角的主题按钮，切到「$1」"
}

shot() {
  ui front "$PID"; sleep 1.5
  /usr/sbin/screencapture -x -l "$WID" "$OUT/$1.png"
  say "  已保存 $1.png"
}

say "启动 Mihomo-Monitor（示例数据）"
MIHOMO_MONITOR_HOME="$DEMO" "$EXE" >/dev/null 2>&1 &
PID=$!
WID=""
for _ in $(seq 1 60); do WID=$(ui window "$PID"); [ -n "$WID" ] && break; sleep 0.5; done
[ -n "$WID" ] || fail "没等到 Mihomo-Monitor 的窗口。如果它刚弹出权限请求，授权后重新运行。"
if [ "$(ui press "$PID" "概览" 0 exact)" != ok ]; then
  say "无法操作 Mihomo-Monitor 的界面：请在「系统设置 → 隐私与安全性 → 辅助功能」里允许当前终端，或用 MANUAL=1 手动切换页面。"
fi
# The live chart needs a little history from the fake core before it looks like anything.
say "等实时速率曲线攒一些数据（约 90 秒）…"
sleep 90
# The theme is kept in the WebView's storage, which the real app shares: put it back at the end.
ORIGINAL=$(ui label "$PID" "主题：" | sed 's/^主题：//')

say "开始截图 → $OUT"
theme 浅色
press "概览" "点左侧「概览」" 0 exact
press "近 30 天" "点「流量」右侧的「近 30 天」"; sleep 1; shot overview-light
theme 深色;                                      shot overview-dark
theme 浅色
press "内核" "点左侧「内核」" 0 exact;           shot sources
press "设置" "点左侧「设置」" 0 exact;           shot settings
[ -n "$ORIGINAL" ] && theme "$ORIGINAL"

say "完成。截图在 $OUT；示例数据留在 $DEMO，可以删除。"
say "记得重新打开你自己的 Mihomo-Monitor。"
