# CC Time Blend との比較

参照した資料は [Cycore FX HD 1.8.9 Manual の CC Time Blend / CC Time Blend FX](https://cycorefx.com/downloads/cfx_hd_std/CycoreFX%20HD%201.8.9%20Manual.pdf)（印刷ページ 130〜133）。公開ソースや演算式は確認できないので、以下は資料に書かれた機能と FeedbackRun の実装を比較したものです。

| Cycore の機能 | FeedbackRun | 差分 |
| --- | --- | --- |
| Instance: Paste / Copy | 2 枚の調整レイヤーで Paste / Copy | 同じ基本構成。1 コンポジション 1 組 |
| Paste 1 個で Time Blend に近い残像 | Single (Time Blend) | 1 枚で順番に入力を蓄積。標準設定は 2 枚 |
| Transfer の 13 種の現行モード | Transfer / Operation | 全 13 種に相当する選択肢を用意。演算式の画素一致は未確認 |
| Classic 転送モード | なし | 旧版互換用。以前の要望に従い省略 |
| Accumulation | Accumulation | RGB とアルファを一緒に減衰。Blend は前後フレームの時間的な混合 |
| Clear | Clear and restart here | AE の時間軸上で開始フレームを変更して再計算。Cycore の即時バッファ消去とは異なる |
| Clear To: Transparent / Current Frame | Clear To (Blend only) | Blend の開始画に適用。初期値は Current Frame |
| 動画・アニメーション入力 | 各時刻の入力を取得 | コード上は対応。AE 実機での現行構成は未確認 |
| 順次再生しかできず AE キャッシュに影響される | 時刻順の再計算とセッション内フレームキャッシュ | ランダムアクセスを想定。AE 実機での安定性は未確認 |
| 画面サイズ変更で蓄積バッファが消える | サイズごとにキャッシュを区別 | ライブ時は再計算。ディスク再生時は同じ解像度の記録が必要 |

FeedbackRun 固有の `Delay`、`History limit`、`Feedback Transform`、`Dry / Wet Mix` は Cycore の同名機能ではありません。`History limit` は指定フレーム数ごとに履歴を始め直すため、その境界で画が切り替わります。`0` は上限なしです。Single では CC Time Blend と同じ 1 フレーム間隔で蓄積し、Delay は使いません。

TouchDesigner の [Feedback TOP](https://docs.derivative.ca/Feedback_TOP) は Target TOP の前フレームを返すノードです。FeedbackRun では Target を上の Copy に固定し、間に AE の処理を置きます。[Composite TOP](https://docs.derivative.ca/Composite_TOP) の全モードや任意の TOP 接続、3D テクスチャは対応していません。

FeedbackRun は独自形式 `.feedbackrun` のフレームを指定フォルダへ保存し、Playback では AE の Smart Render 出力へ直接読み戻します。動画として読み込む方式ではありません。保存されていないフレームや別のプレビュー解像度はエラーになります。

ユーザーの希望により AE 実機テストは行っていません。ビルドとコード確認を完了した状態です。
