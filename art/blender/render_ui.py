"""Render transparent UI portraits from the refined live Blender scene."""
import bpy
from pathlib import Path
from mathutils import Vector

ROOT=Path(__file__).resolve().parents[2]
scene=bpy.context.scene
camera=scene.camera
saved=(camera.matrix_world.copy(),camera.data.type,camera.data.ortho_scale,scene.render.resolution_x,scene.render.resolution_y,scene.render.film_transparent,scene.render.filepath)
visibility={o:o.hide_render for o in scene.objects}
scene.render.film_transparent=True
scene.render.resolution_x=scene.render.resolution_y=256
scene.render.image_settings.file_format='PNG'
scene.render.image_settings.color_mode='RGBA'
out=ROOT/'game/assets/ui'
out.mkdir(parents=True,exist_ok=True)

for name,objects,center,offset,scale in [
    ('ao_umiushi',list(bpy.data.objects['ao_umiushi'].children),(0,1,0.25),(2,-3,2.8),2.55),
    ('medal_normal',[bpy.data.objects['medal_normal_refined']]+[o for o in scene.objects if o.name.startswith('coin_nautilus_stamp')],(0,0,0),(0,-1,2),0.7),
]:
    for o in scene.objects:
        o.hide_render=o.type not in {'CAMERA','LIGHT'} and o not in objects
    # Keep only the authored preview light rig.
    for o in scene.objects:
        if o.type=='LIGHT':
            o.hide_render=visibility[o]
    camera.data.type='ORTHO'
    camera.data.ortho_scale=scale
    camera.location=Vector(center)+Vector(offset)
    camera.rotation_euler=(Vector(center)-camera.location).to_track_quat('-Z','Y').to_euler()
    scene.render.filepath=str(out/(name+'.png'))
    bpy.ops.render.render(write_still=True)

for o,hidden in visibility.items():
    o.hide_render=hidden
camera.matrix_world,camera.data.type,camera.data.ortho_scale,scene.render.resolution_x,scene.render.resolution_y,scene.render.film_transparent,scene.render.filepath=saved
print('Rendered two transparent UI portraits.')
