#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export APP_AGG_OPEN_UI=1
export APP_AGG_CORE_DIR="$DIR/.cache/cliproxyapi"

echo "正在啟動 ModelBridge 本機 AI 聚合閘道桌面視窗..."
exec "$DIR/apps/desktop/src-tauri/target/debug/app-agg-001-desktop"
