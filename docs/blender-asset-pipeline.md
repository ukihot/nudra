# NUDRA：Blender 5.3 でステージとメダルを作り、Bevy へ渡すワークフロー

- 作成日：2026-11-20
- 方針：**Blender 5.3 をステージエディタとして使い、形状と初期配置を1つのステージGLBへ書き出す。**
  Houdini 22 で作る予定だった手順（[houdini-stage-markers.md](houdini-stage-markers.md)・[houdini-asset-brief.md](houdini-asset-brief.md)）は置き換える。
  命名（`root_game` 以下のノード名）・原点・座標の取り決めは引き継ぐ。
- 制作元：[art/blender/nudra_assets.blend](../art/blender/nudra_assets.blend)
- 書き出し：`game/assets/models/stages/stage_01.glb`、`game/assets/models/medals/medal_normal.glb`、
  `game/assets/models/catapult/catapult_{base,chute}.glb`、`game/assets/models/pushers/ao_umiushi.glb`
- 関連：[基本設計 5.8・5.10](basic-design.md)、[stage_01 の定義データ](../game/assets/data/stages/stage_01.stage.ron)

## 1. 座標と単位

右手系、1単位 = ゲーム上の1m。Blender は Z-up、ゲーム（Bevy・glTF）は Y-up。glTF の書き出しで **+Y Up** にすると一致する。

| | ゲーム（Bevy・glTF） | Blender |
|---|---|---|
| 上 | +Y | +Z |
| 奥（敵が来る・画面の上） | −Z | +Y |
| 手前（ゴール・画面の下） | +Z | −Y |
| 例: カタパルトの射出口 | (4.4, 0.85, −4.5) | (4.4, 4.5, 0.85) |

舞台の上面はゲームの Y=0（Blender Z=0）。配置はオブジェクトの位置に置き、頂点へ焼き込まない。回転・負や非一様の拡大は使わない。

## 2. ステージ（コレクション `stage_01`）

すべて Empty `root_game`（原点・回転なし）の子。glTF のノード名 = オブジェクト名で、Bevy は `Name` で識別する。

| ノード | 種類 | ゲームの位置 | 作り方（主なモディファイア） | extras |
|---|---|---|---|---|
| `floor` | メッシュ | (0, −0.25, 0)、8×0.5×16 | Bevel（角度制限・法線を固く）、Weighted Normal | `nudra_kind: floor` |
| `floor_trim` | メッシュ | 側面の金の帯 | Bevel。上面より上に出さない（物理の床と見た目を一致） | — |
| `floor_base` | メッシュ | 舞台の下で奈落へ沈む岩 | Subdivision（Simple）→ Displace（Voronoi）→ Decimate → Weighted Normal | — |
| `rail` | メッシュ（2点の中心線） | (0, 0, −1)、長さ6.8 | **Geometry Nodes `NUDRA_RailTrack`**: Mesh to Curve → Curve to Mesh（四角断面）でレール2本、Resample → Instance on Points で枕木。入力 `Gauge`（軌間）・`Sleepers`（枕木の数） | `nudra_kind: rail`, `rail_id` |
| `bridge_left` / `bridge_right` | メッシュ | (∓2.5, −0.06, −10.5)、長さ5・幅1.6 | 板1枚を **Array**（20枚、0.25m間隔）、Bevel。子の手すり `*_rope` と支柱 `*_posts` は片側を **Mirror**（X）、支柱は Array | `nudra_kind: bridge`, `bridge_id` |
| `spawn_left` / `spawn_right` | メッシュ | (∓2.5, −0.25, −14) | 外の足場（敵の出現）。Bevel | `nudra_kind: spawn`, `bridge_id` |
| `spawn_*_cliff` | メッシュ | 足場の下の崖 | `floor_base` と同じ岩の組み合わせ | — |
| `abyss` | メッシュ | 舞台の16m下 | 暗い発光の面（見た目だけ。物理の奈落は定義データの `abysses`） | `nudra_kind: abyss` |
| `catapult` | Empty（Arrows） | (4.4, 0, −4.5)、盤面の右外・接地原点 | 基準方向は `base_yaw_deg: 90`（ゲームの −X）。回転は持たせない（旋回はBevyが制御） | `nudra_kind: catapult`, `muzzle_height: 0.85`, `base_yaw_deg`, `yaw_range_deg`, `pitch_deg` |
| `goal` | Empty（Circle） | (0, 0, 7.4)、接地原点 | — | `nudra_kind: goal`, `goal_id: 1`, `radius: 1.0` |

- マテリアルは Principled BSDF の値（色・金属度・粗さ・発光）だけを使う（glTF へそのまま出る。手続き型テクスチャは出ない）。
  凹凸は Displace など**形状**で作り、書き出し時にモディファイアを適用する。
- 盤面の外周（左右）は奈落。舞台の側面に壁は作らない（物理の床と一致させる）。

## 3. ノーマルメダル（コレクション `medal_normal`）

| ノード | 三角形 | 用途 |
|---|---|---|
| `medal_normal` | 1,024 | 近景。中央のくぼみ・浮き出た輪・縁 |
| `medal_normal_lod1` | 320 | 大量表示・遠景。輪の模様を省略 |

- 断面（半径0.28・厚さ0.08。ゲームの `normal` の定義と同じ）を1本の折れ線で描き、**Screw** モディファイア（Z軸、32 / 20分割、頂点の結合）で回転体にする。Weighted Normal で縁の陰影を固くする。
- 原点は中心、厚さはゲームの Y 方向（書き出し後の範囲は X・Z が ±0.28、Y が ±0.04）。
- マテリアル `medal_gold`: Metallic 1.0、Roughness 0.22。金属の見え方は環境マップに依存する（基本設計 5.10）。
  Blender のワールドの明暗は確認用で、glTF には出ない。
- 数千枚の表示では LOD1 を基本にする（3,000枚 × 320 ≒ 96万三角形）。近景の判定・切り替えは今後 Presentation で行う。

## 3.5 カタパルト（コレクション `catapult`）

横から盤面へメダルを転がし入れる装置（敵を狙う砲台ではない。基本設計 5.8）。旋回する部分と固定の部分を別のGLBにする。
**原点はどちらも旋回軸＝射出口**（ゲームの `catapult.position`）。前はゲームの −Z（Blender +Y）で、旋回はBevyが掛ける。

| GLB | ルート | 部品（主なモディファイア） |
|---|---|---|
| `catapult_chute.glb` | `catapult_chute`（溝の床） | 後ろ（ゲーム +Z）へ14°で上る長さ1.3の溝。側壁 `*_wall` は **Mirror**（X）、後端の漏斗 `catapult_hopper` は断面を **Screw**＋Solidify、口の飾り輪 `catapult_muzzle`。溝の床は口の0.3m下 |
| `catapult_base.glb` | `catapult_base`（Empty） | 旋回台 `catapult_turntable`（Bevel）と、奈落へ伸びる支柱 `catapult_pillar`（長さ4m） |

マテリアル: `catapult_iron`（Metallic 0.9）と、舞台の縁と同じ `stage_trim`（真鍮）。

## 3.6 Pusherキャラ: アオウミウシ（コレクション `pusher_ao_umiushi`）

ルートは Empty `ao_umiushi`（extras `nudra_kind: pusher`, `character_id`）。**原点はPusherのコライダーの中心**で、底はゲームの Y=−0.42。
体の長軸はゲームの Z（押し幅の向き）、頭は手前（+Z。Blender −Y）を向く。

| ノード | 作り方 |
|---|---|
| `ao_umiushi_body` | 楕円体（幅0.48・長さ1.7・高さ0.6、底を平らに）＋ Subdivision・Weighted Normal。青 `umiushi_blue` |
| `ao_umiushi_mantle` | 縁が22回波打つ楕円の膜（フリル）＋ Solidify・Subdivision |
| `ao_umiushi_stripe_*` / `ao_umiushi_rim` | 背の黄色い縞3本と外套膜の縁取り（カーブのベベル。書き出しでメッシュ化） |
| `ao_umiushi_rhinophore_*` | 橙の触角。ひだのある断面を **Screw** |
| `ao_umiushi_gill` | 橙のえら。羽1枚を **Array**（オブジェクトオフセット `ao_umiushi_gill_pivot`、40°ずつ9枚）で放射状に |
| `ao_umiushi_eye_*` | キャラとしての目とハイライト |

## 4. 書き出し

```text
File > Export > glTF 2.0 (.glb)
  Include:  Selected Objects（コレクションの全オブジェクトを選択）、Custom Properties（extras）
  Transform: +Y Up
  Data > Mesh: Apply Modifiers
  Cameras / Lights: 出さない
```

スクリプトで書き出す場合は `bpy.ops.export_scene.gltf(export_format='GLB', use_selection=True, export_yup=True, export_apply=True, export_extras=True)`。
`medal_normal_lod1` は確認用に横へずらしてあるので、書き出す間だけ原点へ戻す。

書き出し後は GLB の JSON を読み、ノード名・位置・extras が上の表と stage_01 の定義データに一致することを確認する
（2026-11-20 の書き出しで、カタパルト (4.4, 0, −4.5)・ゴール (0, 0, 7.4)・レール (0, 0, −1)・橋 (∓2.5, *, −10.5)・メダルの範囲を確認済み）。

## 5. ゲームへの接続（未実装）

- ステージGLBを読み、名前付きノードと extras から配置を取り出す接続は未実装（基本設計 5.8、12章 No.29）。
  当面は定義データ（`stage_01.stage.ron`）が配置の正本で、GLB は同じ値で作ってある。
- カタパルト（`catapult/base`・`catapult/chute`）とアオウミウシ（`pusher/ao_umiushi`）は `visuals.visuals.ron` で `Model` に接続済み。
- ノーマルメダルは `visuals.visuals.ron` の `"medal/normal"` を `Model(scene: "models/medals/medal_normal.glb#Scene0")` に書き換えれば差し替わる
  （LOD の使い分けは未実装。まず LOD1 の単独シーンでの確認を推奨）。

## 6. 2026-11-20 の洗練パスと接続済みアセット

上の1〜5章は初期制作時の記録。このパスでは原本 `nudra_assets.blend` を保持し、`nudra_refined.blend` を制作元に追加した。
アート方針は「深海の真鍮製メダル筐体」。藍青・青緑・真鍮を基本色、発光シアンを落下と防衛の目印、珊瑚色・紫を敵の識別色にする。

- `refine_assets.py`: 原本から再制作・書き出しする。Blender MCPではファイルを開く操作を単独の呼出で行い、次の呼出でスクリプトの `open_mainfile` 行を除いて実行する（同一呼出で開くとBlenderのUI contextが失効するため）。
- `render_ui.py`: 洗練後のシーンから透過PNG `ui/ao_umiushi.png`（マスコット）と `ui/medal_normal.png` を制作。
- `studio_lighting.py`: 外部素材に依存せず、ソフトボックス照明をHDR cubemapへ積分し、KTX2で書き出す。

| 出力 | 内容・原点 |
|---|---|
| `stages/deck_refined.glb` | 床・金縁・岩・レール・象嵌・落下の発光目印。物理床の中心(0,-0.25,0)が原点 |
| `stages/bridge_refined.glb` | 青緑の板橋と真鍮の装飾。物理側のローカルXが長軸、床中心が原点 |
| `stages/spawn_refined.glb` | 3.2m四方の出現足場。橋のentryにある物理床の中心が原点 |
| `stages/goal_sanctuary.glb` | 貝殻・真珠・真鍮の輪。原点は地面から1m上のGoalセンサー中心 |
| `enemies/{grunt,brute,boss}.glb` | 珊瑚の小型甲殻、紫の重装甲、冠を持つ大型。原点は各カプセル中心 |
| `medals/medal_normal.glb` | 金属の面取り・両面の渦刻印。半径0.28m・厚さ0.08m、1マテリアルの結合メッシュ |
| `pushers/ao_umiushi.glb` | 青・黄・橙の外套膜と真珠色の斑点。物理中心を原点とする |
| `catapult/catapult_{base,chute}.glb` | 深い青の金属・真鍮のリブ・発光する旋回台。射出口を原点とする |

全11GLBを `visuals.visuals.ron` の `Model` に接続済み。ステージ全体のGLBから設備を読み取る移行（No.29）は引き続き別作業で、
現状はRONの物理配置に合わせた各部品の個別接続。床のアセットに橋・Goal・カタパルトを含めず、重複表示を避ける。
床上の縁飾りは薄い象嵌とし、壁のColliderは追加していない。左右全周が落下可能であり、発光部は推奨する押し出し位置を示す。
初期メダル252枚は `layout_medals.py` で個別の位置を生成した不規則な俵積みへ変更し、レール前の山を作る。橋出口を塞ぐ旧配置から移動した。

UIはモデル由来の透過アイコン、Wave・侵入数・撃破数・実ゲージ幅、枠付きのメダル選択欄を使用。日英の文言はi18nへ登録。
描画設定と品質プリセットは[基本設計5.10](basic-design.md#510-レンダリングと映像品質プリセットrender-profile実装済み実機計測は継続)を参照する。
