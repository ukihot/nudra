# Nudra：Houdini 22でステージを作り、Bevyへ配置を渡すワークフロー

- 方針：**Houdiniをステージエディタとして使い、形状と初期配置を1つのステージGLBへ書き出す。**
- 対象：Houdini 22の新しいglTF 2.0入出力、Bevy 0.20（API確認は0.20.1）
- 関連：[アセット制作依頼書](houdini-asset-brief.md)、[基本設計](basic-design.md)

基本の順序は、**部品を作る → 原点を揃える → Pack → 配置 → `path`を付ける → Merge → GLB出力 → 読み直して確認**です。1つのGeometryネットワーク内で作業します。

最初は**床・カタパルト・ゴールの3部品**で往復テストを行い、成功したら橋・出現足場などを追加します。本書は制作と今後の接続実装の手順です。現在のゲームにはステージ全体のGLBから構成を読み取る接続がなく、GLBを置くだけで現行ステージが切り替わる段階ではありません。

## 1. 完成形と納品物

```text
/obj/stage_01（Geometry。OBJの移動・回転0、Scale 1）
  floor_geometry    → pack_floor    → place_floor    → path_floor    ─┐
  catapult_geometry → pack_catapult → place_catapult → path_catapult ─┼→ merge_stage
  goal_geometry     → pack_goal     → place_goal     → path_goal     ─┘      ↓
                                                                      OUT_STAGE
                                                                         ↓
                                                           glTF ROP output driver 2.0
```

各`*_geometry`の前には、多数のモデリングSOPがあって構いません。カタパルトが数千頂点でも、独立した1つのPacked Primitiveにまとめれば、部品単位で配置できます。`OUT_STAGE`は出力を見つけやすくするためのNull SOPです。

納品するのは次の組み合わせです。

| ファイル | 内容 |
|---|---|
| `art/houdini/stage_01.hiplc` | ステージのモデリング・配置・材質・書き出しを編集できるソース |
| `game/assets/models/stages/stage_01.glb` | ステージ全体の形状、材質、名前付きノード、初期Transformを含む1シーン |
| 配置確認画像・検証メモ | ゲームのカメラ角度から見た画像、往復確認結果、Houdini 22のビルド番号 |

ステージ部品の個別GLBと配置専用GLBを必須納品にはしません。メダル・Pusher・敵はゲーム中に生成するため、引き続きステージとは別のアセットです。

## 2. 部品をSOPで作り、原点を揃える

1. `/obj`にGeometryを作って`stage_01`と命名し、中へ入ります。OBJのTranslate / Rotateは0、Scaleは1にします。
2. 床・カタパルト・ゴールをそれぞれ別の枝で作ります。すでにMergeしてあっても、合流前の枝へ戻ってPackを追加すればよく、作り直す必要はありません。
3. 部品をステージ上へ配置する前に、各枝のローカル原点と向きを整えます。

| 部品 | Pack前のローカル形状 |
|---|---|
| 床・橋・出現足場 | 厚みを含む中央を原点にする。厚さ0.5なら上面はY=+0.25 |
| カタパルト | 接地面中央を原点にする。射出方向の基準は-Z、上は+Y |
| ゴール | 接地面中央を原点にする。センサー中心を原点にしない |

右手系、Y-up、1単位=ゲーム上の1mです。形を整えるTransform SOPはPackの前、ステージ上へ置くTransform SOPはPackの後に分けます。寸法調整は原則Pack前に済ませ、配置用Scaleは`(1,1,1)`にします。

## 3. 部品ごとにPackする

1. カタパルトの形状を作った最後のSOPからワイヤーを伸ばし、Tabで`Pack`を検索して追加します。
2. `pack_catapult`など、役割が分かる名前にします。
3. 最初は`Pack By Name`をOFF、`Create Packed Fragments`もOFFとし、枝全体を1つのPacked Geometry Primitiveにまとめます。
4. **`Pivot Location`は`Origin`（原点）にします。** 接地原点を維持したいカタパルト・ゴールで、形状の中央を配置原点にしてしまわないためです。
5. Display Flagを付けて形が保たれていること、Geometry SpreadsheetのPrimitivesでPacked Primitiveが1個になっていることを確認します。

床とゴールにも同じ処理を行います。Packは形状を部品として扱うためのまとまりであり、頂点数を削減する処理ではありません。[Pack SOPの公式説明・パラメータ](https://www.sidefx.com/docs/houdini/nodes/sop/pack.html)

Pack直後は、部品原点が`(0,0,0)`で、配置の回転・Scaleが入っていない状態を基準にします。原点がずれている場合はPack前の形状とPivotを直します。

## 4. Pack後のTransform SOPで配置する

`pack_catapult`の後ろへTransform SOPを追加し、`place_catapult`と命名します。Transform SOP自身のPivotも原点に揃えます。

| 設定 | カタパルトの初回検証値 |
|---|---|
| Translate | `(0, 0, 5.8)` |
| Rotate（度） | `(0, 0, 0)` |
| Scale / Uniform Scale | `(1,1,1)` / `1` |

これで形状を包んだPacked Primitive全体を配置します。**ステージ配置をPack前の頂点位置へ焼き込まず、Pack後のTransformに残す**ことが重要です。

各部品の初回配置は次の値を使えます。ステージ01全体の見た目が揃ってから、Houdiniで調整します。

| 部品 | Translate | Rotate（度） | 補足 |
|---|---|---|---|
| 床 | `(0, -0.25, 0)` | `(0,0,0)` | 寸法8×0.5×16。上面がY=0 |
| カタパルト | `(0, 0, 5.8)` | `(0,0,0)` | 接地原点 |
| ゴール | `(0, 0, 7.4)` | `(0,0,0)` | 接地原点 |
| 左橋 | `(-2.5, -0.25, -10.5)` | `(0,90,0)` | 長手がローカルXの長さ5の橋をZ方向へ向ける |
| 右橋 | `(2.5, -0.25, -10.5)` | `(0,90,0)` | 左右で同じ形状を使ってよい |
| 左出現足場 | `(-2.5, -0.25, -13)` | `(0,0,0)` | 厚さ0.5の中央原点 |
| 右出現足場 | `(2.5, -0.25, -13)` | `(0,0,0)` | 同上 |

負・非一様Scale、シアーは初回の受け渡しでは使いません。ゲーム中のカタパルトの旋回は、ここで決める初期の向きとは別にBevyで制御します。

## 5. Packed Primitiveに`path`を付ける

1. `place_catapult`の後ろにAttribute Wrangleを追加して`path_catapult`と命名します。
2. **Run Overを`Primitives`**にします。
3. 次を入力します。

```c
s@path = "/root_game/catapult";
```

`path`は**外側のPacked Primitiveのstring属性**として設定します。Pack内のポリゴンや、Point属性に付ける手順とは区別してください。Packが自動作成する属性に頼らず、このWrangleで最終値を明示します。

| 部品 | `path` | Bevyで探すノード名 |
|---|---|---|
| 床 | `/root_game/floor` | `floor` |
| カタパルト | `/root_game/catapult` | `catapult` |
| ゴール | `/root_game/goal` | `goal` |
| 左橋 | `/root_game/bridge_left` | `bridge_left` |
| 右橋 | `/root_game/bridge_right` | `bridge_right` |
| 左出現足場 | `/root_game/spawn_left` | `spawn_left` |
| 右出現足場 | `/root_game/spawn_right` | `spawn_right` |
| レールの見た目（後続） | `/root_game/rail` | `rail` |

Geometry Spreadsheetを開き、Primitivesの`path`列を確認します。同じ形状を複製した橋にも、**配置ごとに別のpath**を付けます。末尾の名前もファイル内で一意にします。これによりBevy側で`Name`を使って識別できます。

## 6. Mergeしてステージを組み立てる

1. 各`path_*`の出力をMerge SOPの`merge_stage`へ接続します。
2. 後ろにNull SOPを追加し、`OUT_STAGE`と命名します。
3. `OUT_STAGE`を表示し、ステージ全体の形と配置を確認します。
4. Geometry Spreadsheetで、最初は3個、本制作では7個以上のPacked Primitiveと、それぞれの`path`を確認します。

**Merge後にステージ全体をもう一度Packしたり、Unpackして頂点へ配置を焼き込んだりしません。** 部品ごとのPacked Primitiveと属性を出力まで維持します。カメラ・ライト・寸法ガイドは`OUT_STAGE`へ混ぜません。

## 7. Houdini 22から階層付きGLBを出力する

1. Geometry内でTabから**`glTF ROP output driver 2.0`**を追加します。Houdini 22の新実装を使います。
2. `OUT_STAGE`を入力に接続します。
3. 次の設定にします。

| 設定 | 値 |
|---|---|
| `Output File` | `$HIP/export/stage_01.glb` |
| `Build Hierarchy from Attribute` | **ON** |
| `Path Attribute` | **`path`** |
| `Scene Name` | `stage_01` |
| `Valid Frame Range` | `Render Current Frame`。フレーム1で書き出す |
| `Export Animations to` | `None` |
| `Export Materials` | ON |
| `Export Cameras` / `Export Lights` | OFF |
| `Use Draco Compression` | OFF（初回） |

`Save to Disk`から出力します。`/out`のglTF 2.0 ROPを使う場合は、`Use SOP Path`をONにし、`SOP Path`へ`/obj/stage_01/OUT_STAGE`を指定して同じ階層設定を使います。OBJ Nullを並べる作業は不要です。

公式仕様ではprimitiveのstring属性から階層を作成し、Packed PrimitiveのTransformを出力ノードへ渡せます。パラメータ名は`Build Hierarchy from Attribute`ですが、説明文では`Build Hierarchy from Path Attribute`とも呼ばれています。[Houdini 22 glTF ROP output driver 2.0](https://www.sidefx.com/docs/houdini/nodes/sop/rop_gltf.html)

出力後は概念的に次の構成になります。メッシュが名前付きノード自身に付くか、子の描画Entityになるかに依存せず、ゲーム上の役割は名前付きノードへ対応させます。

```text
stage_01.glb（1シーン）
└─ root_game（移動・回転0、Scale 1）
   ├─ floor          ─ 床のメッシュと配置
   ├─ catapult       ─ カタパルトのメッシュと配置
   ├─ goal           ─ ゴールのメッシュと配置
   ├─ bridge_left    ─ 左橋のメッシュと配置
   ├─ bridge_right   ─ 右橋のメッシュと配置
   ├─ spawn_left     ─ 左出現足場のメッシュと配置
   └─ spawn_right    ─ 右出現足場のメッシュと配置
```

1つのGLBですが、1つに結合した巨大メッシュにはしません。最初は`.gltf`の確認用出力も作ると、`nodes`の名前・Transformをテキストで確認できます。実行用GLBは`game/assets/models/stages/stage_01.glb`へコピーします。

## 8. Houdiniへ読み直して確認する

1. 検証用Geometryを別に作ります。制作元の`OUT_STAGE`とは接続しません。
2. 新しい**glTF 2.0 SOP**を追加し、出力した`stage_01.glb`を指定します。
3. `Load`を`Scene`、`Import Node Geometry As`を`Packed Primitive`にします。
4. `Enable Path Attrib`をON、`Path Attrib`を`path`にします。
5. 形状と配置、Primitivesの`path`、Packed PrimitiveのTransformを確認します。

この読み込みでは、Packed Primitiveに**glTFノードのワールドTransform**が復元されます。`Flattened Geometry`では配置が頂点へ焼かれるため、今回のTransform検証には使いません。[Houdini 22 glTF 2.0 SOP](https://www.sidefx.com/docs/houdini/nodes/sop/gltf.html)

合格条件は、形状が合っていること、`root_game/catapult`などの階層が残ること、接地原点を基準に配置・回転が復元されることです。名前だけ残ってノードが原点のままなら、Pack前後のTransformとPivotを見直します。

なお、公式資料では新しいglTF 2.0 SOPによる**GLB内の埋め込み画像の取り込みは未対応**とされています。Houdiniへの往復では形状・階層・Transformを確認し、材質と画像はBevyで確認します。画像もHoudiniで調べたい場合は外部画像付き`.gltf`を確認用に出力します。[テクスチャ取り込みの制限](https://www.sidefx.com/docs/houdini/nodes/sop/gltf.html)

Houdini 22の実機出力は本書作成時点では未検証です。最初の3部品でこの工程を完了してから、ステージ全体へ広げます。

## 9. Bevy 0.20で読み込み、部品のTransformを取る

### 9.1 読み込みと取得の順序

ステージGLBを**1回だけ**`WorldAssetRoot`で展開します。`WorldInstanceReady`を受けて読み取り待ちにし、`PostUpdate`の`TransformSystems::Propagate`より後に、そのステージの子孫から`Name`を探します。[Bevy 0.20.1の公式使用例](https://github.com/bevyengine/bevy/blob/v0.20.1/examples/gltf/edit_material_on_gltf.rs)

ノードの`Transform`は親基準です。配置の取得には`GlobalTransform`を使い、読み込み元Entityの`GlobalTransform`に対して`reparented_to`することで、ステージ基準のTransformを得ます。ステージ全体の移動・回転を二重適用しないためです。[GlobalTransformの実装](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_transform/src/components/global_transform.rs)

### 9.2 最初の3部品を読む検証用Plugin

`DefaultPlugins`へ追加する例です。取得結果は部品のEntityとステージ基準Transformの組です。結果を消費するステージ生成処理やエラー表示は別途実装します。本制作では設定に応じて橋・足場なども必須ノードへ加えます。掲載コードはBevy 0.20.1で`cargo check`済みです。

```rust
use std::collections::BTreeMap;
use bevy::{
    gltf::{convert_coordinates::GltfConvertCoordinates, GltfLoaderSettings},
    prelude::*,
    world_serialization::WorldInstanceReady,
};

const PARTS: [&str; 3] = ["floor", "catapult", "goal"];

#[derive(Component)]
struct ImportedStage;

#[derive(Component)]
struct PendingStageNodes;

#[derive(Component)]
pub struct StageNodes(pub Result<BTreeMap<String, (Entity, Transform)>, String>);

pub struct StageImportExamplePlugin;

impl Plugin for StageImportExamplePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_stage)
            .add_observer(stage_ready)
            .add_systems(
                PostUpdate,
                collect_stage_nodes.after(TransformSystems::Propagate),
            );
    }
}

fn load_stage(mut commands: Commands, assets: Res<AssetServer>) {
    let scene = assets.load_builder()
        .with_settings(|settings: &mut GltfLoaderSettings| {
            settings.load_cameras = false;
            settings.load_lights = false;
            settings.load_animations = false;
            settings.convert_coordinates = Some(GltfConvertCoordinates {
                rotate_scene_entity: false,
                rotate_meshes: false,
            });
        })
        .load(GltfAssetLabel::Scene(0).from_asset("models/stages/stage_01.glb"));
    commands.spawn((ImportedStage, WorldAssetRoot(scene)));
}

fn stage_ready(
    ready: On<WorldInstanceReady>,
    stages: Query<(), With<ImportedStage>>,
    mut commands: Commands,
) {
    if stages.contains(ready.entity) {
        commands.entity(ready.entity).insert(PendingStageNodes);
    }
}

fn collect_stage_nodes(
    mut commands: Commands,
    stages: Query<(Entity, &GlobalTransform), (With<ImportedStage>, With<PendingStageNodes>)>,
    children: Query<&Children>,
    nodes: Query<(&Name, &GlobalTransform)>,
) {
    for (root, root_global) in &stages {
        let result = (|| {
            let mut parts = BTreeMap::new();
            for entity in children.iter_descendants(root) {
                let Ok((name, global)) = nodes.get(entity) else { continue };
                if !PARTS.contains(&name.as_str()) { continue; }
                let pose = global.reparented_to(root_global);
                if parts.insert(name.as_str().to_owned(), (entity, pose)).is_some() {
                    return Err(format!("duplicate stage node: {name}"));
                }
            }
            for name in PARTS {
                if !parts.contains_key(name) {
                    return Err(format!("missing stage node: {name}"));
                }
            }
            Ok(parts)
        })();
        commands.entity(root)
            .insert(StageNodes(result))
            .remove::<PendingStageNodes>();
    }
}
```

座標変換は両方OFFにし、制作側と合意したY-up / -Z前方をそのまま使います。[座標変換設定](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_gltf/src/convert_coordinates.rs)

この例に加え、本番ではルートのidentity、有限値、単位Scale、必要な部品とメッシュの存在・階層も検証します。`Gltf.named_nodes`から直接読む方法もありますが、`GltfNode.transform`はローカル値なので、親子の変換合成が必要です。最初はシーン展開後の`GlobalTransform`で統一します。[Gltf / GltfNodeの定義](https://github.com/bevyengine/bevy/blob/v0.20.1/crates/bevy_gltf/src/assets.rs)

## 10. 物理判定・ゲーム処理を接続する

| GLBノード | ゲーム側で接続するもの |
|---|---|
| `floor` | 単純形状の床Collider。厚み中央のTransformを使用 |
| `bridge_left` / `bridge_right` | 橋Colliderと進入経路の対応 |
| `spawn_left` / `spawn_right` | 足場Colliderと敵の出現設定の対応 |
| `catapult` | 射出装置の基準位置・初期の向き。ゲーム中の旋回はBevyで制御 |
| `goal` | ゴール到達センサー。見た目は接地位置、判定中心だけ高さの半分を加算 |

見た目のメッシュをそのまま物理判定に使う必要はありません。寸法・判定半径・射出速度・難易度・Wave・橋の端点やレール経路などのゲーム設定は、取り込み実装に合わせて設定データと対応付けます。メッシュ名だけから全ゲームルールが自動生成されるとは扱いません。

ステージGLBにはすでに床・カタパルト・ゴールの見た目があります。**現行の`Appearance`経由で同じ設備のモデルを追加生成しません。** 読み込んだノード・描画子Entityを使い、物理やゲーム用Entityと対応付けます。カタパルトを動かす場合は、初期Transformを取得した後に対応ノードを制御するか、見た目の階層を保持してゲーム用Entityへ付け替えます。付け替え時も同じ配置を二重適用しないようにします。

### 射出口は次の段階で追加する

最初の3部品の往復テストには、空のMarkerは必要ありません。カタパルトの配置が渡ることを先に確認します。

後続では`/root_game/catapult/socket_muzzle`を、射出位置・方向を表すノードとして追加する方針です。ただし**空のPacked Primitiveがそのまま空ノードとして保持されるとは仮定しません。** SOPからの書き出し方法と、動く親に対するローカルTransformを小さなテストで確認してから制作手順へ追加します。`path`を付けただけで空の射出口Markerが完成したとは扱いません。

接続前の射出点は従来のローカルオフセット`(0,0.6,0)`、仰角12度をゲーム設定として使えます。`socket_muzzle`へ接続した段階で、このオフセット・角度を重ねて加算しないように切り替えます。

### 制作とプログラムの役割

- **Houdini / ステージGLB**：形状と初期配置の正本。再配置はSOPの配置用Transformから行い、ステージ全体を再出力します。
- **ステージ設定**：GLBのパス、ノード名と設備IDの対応、寸法・判定・ゲームルール。GLB接続後は同じ設備の初期位置・回転を別途手入力して二重管理しません。
- **Presentation / Infrastructure**：展開済みノードとTransformを`game/src/port.rs`の境界経由で共有し、表示と物理・ゲーム設定を接続します。互いの実装を直接参照しません。
- **Domain / Application**：BevyのEntity・Handle・GLBへ依存せず、変換・検証済みのゲーム用の値を受け取ります。ゲーム中の位置・速度はECS / Avianを正本とします。

現行ステージは接続実装が完了するまで従来の定義で動作します。GLBを採用するステージは読み込み・検証成功を開始条件とし、欠落時は原因を表示します。

## 11. 最初の往復テストの完了チェック

- [ ] 床・カタパルト・ゴールを独立したSOPの枝で作った
- [ ] 原点と向きを揃え、各枝を1つずつPackした
- [ ] PackのPivotを原点へ揃えた
- [ ] Pack後のTransformで配置し、配置用Scaleを1にした
- [ ] 外側のPrimitiveに一意な`path`を付けた
- [ ] Merge後も部品ごとのPacked Primitiveが残っている
- [ ] 階層出力をONにして、ステージ全体を1つのGLBへ出力した
- [ ] Houdiniで読み直し、形状・階層・配置を確認した
- [ ] Bevyで`floor=(0,-0.25,0)`、`catapult=(0,0,5.8)`、`goal=(0,0,7.4)`を取得できた
- [ ] カタパルトをY軸回転して再出力し、Bevyでも同じ回転を取得できた
- [ ] 読み込み元Entityを移動・回転しても、ステージ基準の取得値は変わらない
- [ ] Bevyで材質・画像を確認し、設備の見た目が二重生成されていない

この確認が済んだら橋・出現足場を追加し、物理判定とゲーム処理を接続します。射出口ノードの検証はその後の工程です。
