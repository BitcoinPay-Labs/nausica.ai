# Upfile Protocol

BSVブロックチェーンを使用したファイルのオンチェーン保存・復元システム

## 概要

Upfile Protocolは、ファイルをBSV（Bitcoin SV）ブロックチェーンに永続的に保存し、トランザクションID（TXID）を使用して復元できるウェブアプリケーションです。

## 機能

- **Upload**: ファイルをBSVブロックチェーンに保存
  - ファイル選択 → コスト計算 → BSV支払い → オンチェーン保存
  - QRコード対応の支払い画面
  - 自動支払い検知

- **Download**: TXIDからファイルを復元
  - Manifest TXIDを入力 → ブロックチェーンからデータ取得 → ファイル復元

- **Dashboard**: ジョブ履歴管理
  - アップロード/ダウンロードの履歴表示
  - ステータス確認

## 技術スタック

| コンポーネント | 技術 |
|---------------|------|
| バックエンド | Rust (Axum) |
| データベース | SQLite |
| ブロックチェーン連携 | Bitails API |
| フロントエンド | HTML/CSS/JavaScript + Lucide Icons |

## セットアップ

### 必要条件

- Rust 1.70+
- SQLite

### インストール

```bash
# リポジトリをクローン
git clone https://github.com/i4RP/OriginalDataArchitecture.git
cd OriginalDataArchitecture

# 環境変数を設定
cp .env.example .env
# .envファイルを編集してBitails APIキーを設定

# ビルド
cargo build --release

# 実行
./target/release/upfile-protocol
```

### 環境変数

```
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
DATABASE_URL=./data/upfile.db
BITAILS_API_URL=https://api.bitails.io
BITAILS_API_KEY=your_api_key_here
FEE_RATE=2
```

## API エンドポイント

| メソッド | パス | 説明 |
|---------|------|------|
| GET | `/` | Dashboard |
| GET | `/upload` | Upload画面 |
| POST | `/prepare_upload` | アップロード準備 |
| GET | `/download` | Download画面 |
| POST | `/start_download` | ダウンロード開始 |
| GET | `/status/{job_id}` | ステータス画面 |
| GET | `/status_update/{job_id}` | ステータスAPI |
| GET | `/download_file/{job_id}` | ファイルダウンロード |

## ライセンス

MIT License

## 作者

i4RP
