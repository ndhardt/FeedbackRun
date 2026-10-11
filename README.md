# FeedbackRun

After Effects で、画を次のフレームへ戻すエフェクトです。2 枚の調整レイヤーの間に置いたエフェクトやレイヤーも、繰り返し処理できます。

## 使い方

コンポジションを上からこの順に並べます。

```text
調整レイヤー：FeedbackRun → Copy
繰り返したいエフェクトやレイヤー
調整レイヤー：FeedbackRun → Paste
元の素材（動画も可）
```

両方の調整レイヤーに FeedbackRun を入れ、最初の `Instance (Mode)` だけを切り替えます。設定は上の Copy で行います。1 枚で残像を作る場合は `Single (Time Blend)` を選びます。

`Transfer / Operation` で合成方法を選びます。`Accumulation` は前フレームを残す割合で、100% なら減衰しません。`Blend` は 100% だと新しい画が入らないため、90% 前後から調整してください。`Clear To` は Blend の開始画を透明または現在のフレームから選びます。`Clear and restart here` は再生ヘッド位置から履歴を始め直します。

計算済みフレームは AE 起動中に最大 1 GiB 保存します。1 コンポジションにつき 1 組の FeedbackRun を使ってください。

## ディスクキャッシュ

Copy（または Single）の `Cache Folder` で専用のフォルダを選び、`Disk Cache` を `Record to Disk` にします。保存したい範囲をプレビューまたはレンダリングすると、計算された各フレームが `.feedbackrun` ファイルとして保存されます。`Playback from Disk` に切り替えると、FeedbackRun が保存済みフレームを AE のエフェクト出力として直接返します。動画の読み込みは不要です。既存のフォルダを選ぶと最新の記録を読み込みます。

未保存のフレームや、解像度・色深度・フレーム設定が異なる記録は読めません。その設定で再度記録してください。元のアニメーションを変更したら、`Clear and restart here` で新しい記録を始めます。`Delete cached frames` は選択フォルダ内の FeedbackRun キャッシュを削除します。キャッシュファイルはプロジェクトとは別に保存されるので、使い続ける間はフォルダを移動しないでください。

## インストール

[Releases](https://github.com/ndhardt/FeedbackRun/releases) から使用する OS の zip をダウンロードし、展開してください。AE を終了して、macOS は `FeedbackRun.plugin`、Windows は `FeedbackRun.aex` を Adobe の共通プラグインフォルダ `MediaCore` に入れ、AE を再起動します。macOS 版はアドホック署名で、公証は行っていません。

## ビルド

Rust と just を用意し、macOS では `NO_INSTALL=1 just release`、Windows の PowerShell では `$env:NO_INSTALL=1; just release` を実行します。別の After Effects SDK でバインディングを生成する場合は `AESDK_ROOT` を指定してください。

ライセンス: MIT または Apache-2.0。
