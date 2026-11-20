"""Rebuild the art pass from the original .blend; run inside Blender (also via MCP).

All runtime exports are evaluated, joined meshes in physics-local coordinates.
The original source stays untouched. No downloaded or generated third-party art.
"""
import bpy
import math
import random
from pathlib import Path
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[2]
bpy.ops.wm.open_mainfile(filepath=str(ROOT / 'art/blender/nudra_assets.blend'))
scene = bpy.context.scene
random.seed(27)


def material(name, color, metal=0.0, rough=0.4, glow=0.0):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    m.use_nodes = True
    p = next(n for n in m.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
    p.inputs['Base Color'].default_value = (*color, 1)
    p.inputs['Metallic'].default_value = metal
    p.inputs['Roughness'].default_value = rough
    p.inputs['Emission Color'].default_value = (*color, 1)
    p.inputs['Emission Strength'].default_value = glow
    m.diffuse_color = (*color, 1)
    return m


gold = material('stage_trim', (0.68, 0.38, 0.105), 0.78, 0.27)
coin_gold = material('medal_gold', (0.83, 0.51, 0.14), 0.92, 0.23)
deck = material('stage_deck', (0.012, 0.045, 0.075), 0.35, 0.37)
iron = material('catapult_iron', (0.024, 0.065, 0.085), 0.72, 0.29)
rock = material('cliff_rock', (0.018, 0.042, 0.063), 0.1, 0.82)
material('bridge_wood', (0.085, 0.24, 0.245), 0.3, 0.43)
material('bridge_rope', (0.47, 0.28, 0.08), 0.7, 0.35)
material('rail_steel', (0.12, 0.3, 0.32), 0.8, 0.22)
material('umiushi_blue', (0.012, 0.17, 0.63), 0.05, 0.22)
material('umiushi_yellow', (1.0, 0.58, 0.065), 0.15, 0.28)
material('umiushi_orange', (0.95, 0.23, 0.055), 0.0, 0.3)
cyan = material('biolume_cyan', (0.06, 0.8, 0.72), 0.15, 0.25, 2.5)
amber = material('biolume_amber', (1.0, 0.38, 0.06), 0.1, 0.28, 1.6)
pearl = material('pearl_porcelain', (0.65, 0.89, 0.83), 0.18, 0.18)
ink = material('creature_ink', (0.004, 0.012, 0.027), 0.05, 0.23)
shell = material('shell_coral', (0.72, 0.19, 0.13), 0.1, 0.34)
violet = material('shell_violet', (0.27, 0.08, 0.34), 0.35, 0.3)

art = bpy.data.collections.new('refined_stage')
scene.collection.children.link(art)


def finish(o, name, mat, col=art, parent=None):
    o.name = name
    for c in list(o.users_collection):
        c.objects.unlink(o)
    col.objects.link(o)
    if mat:
        o.data.materials.clear()
        o.data.materials.append(mat)
    if parent:
        o.parent = parent
    return o


def box(name, pos, size, mat, bevel=0.025, parent=None):
    bpy.ops.mesh.primitive_cube_add(size=1, location=pos)
    o = finish(bpy.context.view_layer.objects.active, name, mat, parent=parent)
    o.scale = size
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    if bevel:
        b = o.modifiers.new('Soft machined edges', 'BEVEL')
        b.width, b.segments = bevel, 2
    return o


def uv(name, pos, size, mat, parent=None):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=16, ring_count=8, radius=1, location=pos)
    o = finish(bpy.context.view_layer.objects.active, name, mat, parent=parent)
    o.scale = size
    for f in o.data.polygons:
        f.use_smooth = True
    return o


def line(name, points, radius, mat, parent=None):
    c = bpy.data.curves.new(name, 'CURVE')
    c.dimensions, c.bevel_depth, c.bevel_resolution = '3D', radius, 1
    s = c.splines.new('POLY')
    s.points.add(len(points) - 1)
    for p, xyz in zip(s.points, points):
        p.co = (*xyz, 1)
    o = bpy.data.objects.new(name, c)
    art.objects.link(o)
    o.data.materials.append(mat)
    o.parent = parent
    return o


def ring(name, pos, radius, tube, mat, parent=None, segments=48):
    pts = [(pos[0]+radius*math.cos(i*math.tau/segments), pos[1]+radius*math.sin(i*math.tau/segments), pos[2]) for i in range(segments+1)]
    return line(name, pts, tube, mat, parent)


# A flush inlaid deck: every edge remains open, matching the unchanged colliders.
for x in [-3.86, 3.86]:
    for ya, yb in [(-7.7, -2.4), (-0.2, 3.0), (5.1, 7.7)]:
        box('perimeter_brass_inlay', (x, (ya+yb)/2, 0.002), (0.035, yb-ya, 0.008), gold, 0.005)
    for y in [-1.3, 4.05]:
        box('drop_gap_light', (x, y, 0.005), (0.07, 1.85, 0.012), cyan, 0.005)
        for k in range(5):
            o = box('drop_gap_chevron', (x-math.copysign(0.17,x), y-0.65+k*0.31, 0.008), (0.16, 0.055, 0.008), gold, 0.005)
            o.rotation_euler.z = math.copysign(-0.55, x)
        # Recessed brass machinery below the drop lip, never a false wall.
        box('drop_collar', (x, y, -0.18), (0.18, 2.1, 0.16), iron)
        for k in [-0.7, 0, 0.7]:
            box('drop_collar_lamp', (x+math.copysign(0.1,x), y+k, -0.18), (0.025,0.16,0.06), cyan, 0.008)
for y in [-7.72, 7.72]:
    box('end_brass_inlay', (0,y,0.002), (7.75,0.035,0.008), gold, 0.005)
# Sparse contour lines and concentric wave engraving, rather than a grid.
for radius in [1.0, 1.28, 1.9]:
    pts = [(radius*math.cos(t), -6.8+radius*math.sin(t), 0.009) for t in [i*math.pi/40 for i in range(41)]]
    line('sanctuary_wave_inlay', pts, 0.012, gold)
for x in [-1.4, 1.4]:
    line('defense_lane_inlay', [(x,-4.4,0.008),(x,-2.8,0.008),(x*0.85,-2.4,0.008)], 0.012, gold)
for x in [-3.6,3.6]:
    for y in [-7.4,-3.0,0.3,6.8,7.4]:
        uv('deck_rivet',(x,y,0.0),(0.045,0.045,0.018),gold)
# Larger tapered island foundation and layered brass chassis.
base = bpy.data.objects['floor_base']
base.scale.x *= 1.9
base.scale.y *= 1.85
box('chassis_underband',(0,0,-0.49),(8.05,16.05,0.12),iron,0.06)
for x in [-3.97,3.97]:
    box('chassis_brass_band',(x,0,-0.38),(0.035,15.85,0.04),gold,0.008)
# Add organic silhouettes below the surface, behind the action.
for side in [-1,1]:
    for j in range(4):
        x,y,z=side*(4.3+random.random()), random.uniform(-4,6),random.uniform(-2.6,-1.1)
        for k in range(3):
            line('abyss_coral',[(x,y,z),(x+side*0.15,y+0.15*k,z+0.35),(x+side*(0.2+k*0.12),y+0.1*k,z+0.65+0.2*k)],0.045,cyan if j%2==0 else violet)

# Hero: strengthen its silhouette and add small luminous mantle spots.
hero=bpy.data.objects['ao_umiushi']
for o in list(hero.children):
    o.scale.x *= 1.08
for sign in [-1,1]:
    for j in range(7):
        y=-0.52+j*0.17
        uv('ao_umiushi_mantle_spot',(sign*(0.24+0.04*math.cos(y*4)),y,-0.31),(0.027,0.035,0.016),pearl,hero)

# New sanctuary: a pearl cradled by a ribbed scallop, fits the goal's 1m radius.
goal_col=bpy.data.collections.new('goal_sanctuary')
scene.collection.children.link(goal_col)
goal_root=bpy.data.objects.new('goal_sanctuary',None)
goal_col.objects.link(goal_root)
goal_root.location=(0,-7.4,0)
goal_start=set(art.objects)
uv('sanctuary_lower_shell',(0,0,0.03),(0.84,0.62,0.14),pearl,goal_root)
uv('sanctuary_pearl',(0,0.02,0.32),(0.28,0.28,0.28),cyan,goal_root)
for i in range(11):
    a=math.pi*(0.12+0.76*i/10)
    x=math.cos(a)*0.75
    line('sanctuary_shell_rib',[(x,-0.08,0.08),(x*0.85,0.27,0.24),(x*0.65,0.5,0.45),(x*0.5,0.56,0.62)],0.05,pearl,goal_root)
ring('sanctuary_gold_halo',(0,0,0.012),0.94,0.024,gold,goal_root)
goal_objects=list(set(art.objects)-goal_start)

# A readable family of invaders: coral scuttler, armored violet brute, crowned king.
enemy_exports=[]
for idx,(key,r,h,mat) in enumerate([('grunt',0.3,0.9,shell),('brute',0.5,1.4,violet),('boss',0.7,1.8,shell)]):
    col=bpy.data.collections.new('enemy_'+key)
    scene.collection.children.link(col)
    root=bpy.data.objects.new('enemy_'+key,None)
    col.objects.link(root)
    root.location=(-6-idx*2.0,0,h/2)
    before=set(art.objects)
    uv(key+'_body',(0,0,-h*0.05),(r*0.82,r*0.72,h*0.32),mat,root)
    uv(key+'_carapace',(0,0.06,h*0.08),(r*0.9,r*0.72,h*0.25),mat,root)
    for sign in [-1,1]:
        for j in range(3):
            y=-r*0.45+j*r*0.45
            line(key+'_leg',[(sign*r*0.55,y,-h*0.17),(sign*r*0.95,y+r*0.2,-h*0.28),(sign*r*0.95,y-r*0.1,-h*0.48)],r*0.09,ink,root)
        line(key+'_eye_stalk',[(sign*r*0.38,-r*0.38,h*0.12),(sign*r*0.43,-r*0.54,h*0.32)],r*0.07,gold,root)
        uv(key+'_eye',(sign*r*0.43,-r*0.55,h*0.32),(r*0.19,r*0.17,r*0.2),pearl,root)
        uv(key+'_pupil',(sign*r*0.43,-r*0.7,h*0.32),(r*0.09,r*0.06,r*0.12),ink,root)
        uv(key+'_claw',(sign*r*0.7,-r*0.5,-h*0.05),(r*0.24,r*0.26,r*0.22),mat,root)
    for j in range(3+idx*2):
        x=(j-(2+idx*2)/2)*r*0.22
        uv(key+'_shell_stud',(x,0.04,h*0.29),(r*0.08,r*0.1,r*0.09),gold,root)
    if idx==2:
        for j in range(5):
            a=j*math.tau/5
            bpy.ops.mesh.primitive_cone_add(vertices=6,radius1=0.075,radius2=0.012,depth=0.22,location=(0.23*math.cos(a),0.23*math.sin(a),h*0.4))
            finish(bpy.context.view_layer.objects.active,'king_crown',gold,parent=root)
    enemy_exports.append((key,root,list(set(art.objects)-before)))

# A clean, low-cost minted coin. The radial profile gives actual relief on both faces.
def minted_coin():
    n=20
    profile=[(0,-0.032),(0.19,-0.032),(0.21,-0.026),(0.24,-0.026),(0.253,-0.04),(0.272,-0.04),(0.28,-0.029),(0.28,0.029),(0.272,0.04),(0.253,0.04),(0.24,0.026),(0.21,0.026),(0.19,0.032),(0,0.032)]
    verts=[(r*math.cos(i*math.tau/n),r*math.sin(i*math.tau/n),z) for r,z in profile for i in range(n)]
    faces=[(j*n+i,j*n+(i+1)%n,(j+1)*n+(i+1)%n,(j+1)*n+i) for j in range(len(profile)-1) for i in range(n)]
    me=bpy.data.meshes.new('minted_profile')
    me.from_pydata(verts,[],faces)
    me.update()
    o=bpy.data.objects.new('medal_normal_refined',me)
    art.objects.link(o)
    me.materials.append(coin_gold)
    return o

coin=minted_coin()
coin_parts=[coin]
for z in [-0.030,0.030]:
    # Stylized nautilus stamped into both sides, made from the same gold material.
    pts=[]
    for i in range(20):
        t=i/19*math.tau*1.45
        r=0.012+0.012*t
        pts.append((r*math.cos(t),r*math.sin(t),z))
    stamp=line('coin_nautilus_stamp',pts,0.009,coin_gold)
    stamp.data.bevel_resolution=0
    coin_parts.append(stamp)
for o in bpy.data.collections['medal_normal'].objects:
    o.hide_render=True
    o.hide_set(True)

# Refine the chute with warm ribs and a turquoise mouth rim.
chute=bpy.data.objects['catapult_chute']
for y in [-0.25,-0.55,-0.85,-1.1]:
    box('chute_brass_rib',(0,y,-0.27-y*0.24),(0.19,0.025,0.018),gold,0.008,chute)
ring('turntable_light',(0,0,-0.325),0.28,0.018,cyan,bpy.data.objects['catapult_base'])


def export_meshes(objects, path, origin=(0,0,0), rot=None):
    """Flatten evaluated geometry, preserving materials, into one runtime mesh."""
    bpy.context.view_layer.update()
    deps=bpy.context.evaluated_depsgraph_get()
    vertices, faces, indices, smooth, mats = [], [], [], [], []
    local=Matrix.Translation(Vector(origin)).inverted()
    if rot:
        local=rot @ local
    for src in objects:
        if src.type not in {'MESH','CURVE'}:
            continue
        me=bpy.data.meshes.new_from_object(src.evaluated_get(deps),depsgraph=deps)
        me.transform(local @ src.matrix_world)
        offset=len(vertices)
        vertices.extend(tuple(v.co) for v in me.vertices)
        slot_map=[]
        for m in me.materials:
            if m not in mats:
                mats.append(m)
            slot_map.append(mats.index(m))
        for p in me.polygons:
            faces.append(tuple(offset+i for i in p.vertices))
            indices.append(slot_map[p.material_index] if slot_map else 0)
            smooth.append(p.use_smooth)
        bpy.data.meshes.remove(me)
    mesh=bpy.data.meshes.new('runtime_joined')
    mesh.from_pydata(vertices,[],faces)
    mesh.update()
    for m in mats:
        mesh.materials.append(m)
    for p,idx,sm in zip(mesh.polygons,indices,smooth):
        p.material_index,p.use_smooth=idx,sm
    ob=bpy.data.objects.new(Path(path).stem,mesh)
    scene.collection.objects.link(ob)
    bpy.ops.object.select_all(action='DESELECT')
    ob.select_set(True)
    bpy.context.view_layer.objects.active=ob
    out=ROOT/'game/assets'/path
    out.parent.mkdir(parents=True,exist_ok=True)
    bpy.ops.export_scene.gltf(filepath=str(out),export_format='GLB',use_selection=True,export_yup=True,export_animations=False,export_extras=True,export_lights=False,export_cameras=False)
    ob.data.calc_loop_triangles()
    print('EXPORT',path,'triangles',len(ob.data.loop_triangles))
    bpy.data.objects.remove(ob,do_unlink=True)


floor_art=[o for o in art.objects if o.parent is None and o not in coin_parts]
export_meshes([bpy.data.objects[n] for n in ['floor','floor_trim','floor_base','rail']]+floor_art,'models/stages/deck_refined.glb',(0,0,-0.25))
bridge=bpy.data.objects['bridge_left']
# Runtime bridge is X-long and rotated from +X towards entry->exit; Blender source is Y-long.
export_meshes([bridge]+list(bridge.children),'models/stages/bridge_refined.glb',(-2.5,10.5,-0.25),Matrix.Rotation(math.pi/2,4,'Z'))
# Runtime spawn collider is 3.2 x 0.5 x 3.2, centered at entry, not the old 14m locator.
spawn=box('spawn_platform_refined',(-2.5,13,-0.25),(3.2,3.2,0.5),deck,0.09)
export_meshes([spawn,bpy.data.objects['spawn_left_cliff']],'models/stages/spawn_refined.glb',(-2.5,13,-0.25))
# Sensor center is 1m above ground, unlike the Blender goal locator.
export_meshes(goal_objects,'models/stages/goal_sanctuary.glb',(0,-7.4,1))
export_meshes(coin_parts,'models/medals/medal_normal.glb')
export_meshes(list(hero.children),'models/pushers/ao_umiushi.glb',tuple(hero.location))
for name in ['catapult_base','catapult_chute']:
    obj=bpy.data.objects[name]
    export_meshes([obj]+list(obj.children),'models/catapult/'+name+'.glb',tuple(obj.location))
for key,root,objects in enemy_exports:
    export_meshes(objects,'models/enemies/'+key+'.glb',tuple(root.location))
for (_,root,_),pos in zip(enemy_exports,[(-2.5,9,0.45),(2.5,8.5,0.7),(2.5,12,0.9)]):
    root.location=pos

# Presentation lighting for preview only; runtime illumination lives in RenderProfile.
for o in list(scene.objects):
    if o.type=='LIGHT':
        o.hide_render=True
for name,loc,power,color,size in [('Key',(-5,-4,12),2400,(1,0.81,0.57),9),('Sea rim',(5,7,8),3000,(0.23,0.7,1),8),('Fill',(-6,8,7),1700,(0.34,1,0.82),7)]:
    ld=bpy.data.lights.new(name,'AREA')
    ld.energy,ld.color,ld.shape,ld.size=power,color,'DISK',size
    ob=bpy.data.objects.new(name,ld)
    scene.collection.objects.link(ob)
    ob.location=loc
    ob.rotation_euler=(Vector((0,1,0))-ob.location).to_track_quat('-Z','Y').to_euler()
world=scene.world
world.use_nodes=True
bg=next(n for n in world.node_tree.nodes if n.type=='BACKGROUND')
bg.inputs[0].default_value=(0.035,0.09,0.12,1)
bg.inputs[1].default_value=0.45
scene.camera.location=(14,-23,30)
scene.camera.rotation_euler=(Vector((0,2,0))-scene.camera.location).to_track_quat('-Z','Y').to_euler()
scene.camera.data.type='ORTHO'
scene.camera.data.ortho_scale=30
scene.render.resolution_x,scene.render.resolution_y=1440,1080
scene.render.resolution_percentage=100

# A reproducible art review composition (coins are linked display instances only).
preview=bpy.data.collections.new('preview_only')
scene.collection.children.link(preview)
for j in range(140):
    o=bpy.data.objects.new('preview_coin',coin.data)
    preview.objects.link(o)
    o.location=(random.uniform(-3.35,3.35),random.uniform(1.8,5.5),0.06+(j//55)*0.085)
    o.rotation_euler=(random.uniform(-0.12,0.12),random.uniform(-0.12,0.12),random.random()*6.28)
coin.hide_render=True
for o in coin_parts:
    o.hide_render=True
preview.hide_viewport=False
bpy.ops.wm.save_as_mainfile(filepath=str(ROOT/'art/blender/nudra_refined.blend'))
print('Saved refined art source; original source preserved.')

