#!/usr/bin/env python3
"""Procedural ChastiTube icon renderer.  Run from Blender with -- --variant rust."""
import argparse
import math
import os
import sys
import time

import bpy
from mathutils import Vector

ROOT = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(ROOT, "renders")
VARIANTS = ("rust", "black", "zinc", "enamel")


def args():
    p = argparse.ArgumentParser()
    p.add_argument("--variant", choices=VARIANTS, required=True)
    return p.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])


def smooth(obj):
    if obj.type == 'MESH':
        for poly in obj.data.polygons:
            poly.use_smooth = True


def bevel(obj, width, segments=3):
    mod = obj.modifiers.new("soft manufactured edges", 'BEVEL')
    mod.width, mod.segments = width, segments
    mod.limit_method = 'ANGLE'


def metal(name, base, rough=.35, accent=None, weather=.25, paint=False):
    """PBR metal with procedural mottle, wear and grime -- no bitmap textures."""
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    n, l = m.node_tree.nodes, m.node_tree.links
    n.clear()
    out = n.new('ShaderNodeOutputMaterial')
    bs = n.new('ShaderNodeBsdfPrincipled')
    bs.inputs['Metallic'].default_value = 1.0
    bs.inputs['Roughness'].default_value = rough
    tex = n.new('ShaderNodeTexNoise'); tex.inputs['Scale'].default_value = 4.2; tex.inputs['Detail'].default_value = 5.0; tex.inputs['Roughness'].default_value = .7
    fine = n.new('ShaderNodeTexNoise'); fine.inputs['Scale'].default_value = 35; fine.inputs['Detail'].default_value = 3.0
    ramp = n.new('ShaderNodeValToRGB')
    ramp.color_ramp.elements[0].position = .22; ramp.color_ramp.elements[0].color = (*base, 1)
    ramp.color_ramp.elements[1].position = .78; ramp.color_ramp.elements[1].color = (*(accent or tuple(min(1, x * 1.35 + .04) for x in base)), 1)
    l.new(tex.outputs['Fac'], ramp.inputs['Fac'])
    # Broad low-frequency mottling is visible in base color and roughness.
    mix = n.new('ShaderNodeMixRGB'); mix.blend_type = 'MULTIPLY'; mix.inputs[0].default_value = .42
    l.new(ramp.outputs['Color'], mix.inputs[1]); l.new(fine.outputs['Fac'], mix.inputs[2]); l.new(mix.outputs['Color'], bs.inputs['Base Color'])
    roughmath = n.new('ShaderNodeMath'); roughmath.operation = 'MULTIPLY_ADD'; roughmath.inputs[1].default_value = .22; roughmath.inputs[2].default_value = rough - .10
    l.new(fine.outputs['Fac'], roughmath.inputs[0]); l.new(roughmath.outputs[0], bs.inputs['Roughness'])
    bump = n.new('ShaderNodeBump'); bump.inputs['Strength'].default_value = .18; bump.inputs['Distance'].default_value = .06
    l.new(fine.outputs['Fac'], bump.inputs['Height']); l.new(bump.outputs['Normal'], bs.inputs['Normal'])
    if weather > 0:
        grime = n.new('ShaderNodeTexNoise'); grime.inputs['Scale'].default_value = 2.1; grime.inputs['Detail'].default_value = 6.0
        gr = n.new('ShaderNodeValToRGB'); gr.color_ramp.elements[0].position = .54; gr.color_ramp.elements[1].position = .82
        gr.color_ramp.elements[0].color = (0, 0, 0, 1); gr.color_ramp.elements[1].color = (weather, weather, weather, 1)
        dark = n.new('ShaderNodeMixRGB'); dark.blend_type = 'MIX'; dark.inputs[2].default_value = (.28, .065, .012, 1) if name.startswith('rust') else (.055, .047, .035, 1)
        l.new(grime.outputs['Fac'], gr.inputs['Fac']); l.new(gr.outputs['Color'], dark.inputs[0]); l.new(mix.outputs['Color'], dark.inputs[1]); l.new(dark.outputs['Color'], bs.inputs['Base Color'])
    if paint:
        # Convex edge mask plus noise reveals bright bare metal in chips.
        geo = n.new('ShaderNodeNewGeometry')
        edge = n.new('ShaderNodeValToRGB'); edge.color_ramp.elements[0].position = .18; edge.color_ramp.elements[1].position = .55
        chip = n.new('ShaderNodeMath'); chip.operation = 'MULTIPLY'
        coat = n.new('ShaderNodeMixRGB'); coat.inputs[2].default_value = (.32, .18, .09, 1)
        l.new(geo.outputs['Pointiness'], edge.inputs['Fac']); l.new(edge.outputs['Color'], chip.inputs[0]); l.new(fine.outputs['Fac'], chip.inputs[1]); l.new(chip.outputs[0], coat.inputs[0]); l.new(mix.outputs['Color'], coat.inputs[1]); l.new(coat.outputs['Color'], bs.inputs['Base Color'])
    l.new(bs.outputs['BSDF'], out.inputs['Surface'])
    return m


def annular_band(name, z, h, outer, inner, material):
    # Manifold annular rectangular strap with four faces per angular segment.
    seg = 96; verts = []; faces = []
    for zz in (z-h/2, z+h/2):
        for r in (outer, inner):
            for i in range(seg):
                a = 2*math.pi*i/seg; verts.append((r*math.cos(a), r*math.sin(a), zz))
    # layers: bottom outer, bottom inner, top outer, top inner
    for i in range(seg):
        j=(i+1)%seg
        faces += [(i,j,seg+j,seg+i), (2*seg+i,3*seg+i,3*seg+j,2*seg+j), (i,2*seg+i,2*seg+j,j), (seg+i,seg+j,3*seg+j,3*seg+i)]
    me=bpy.data.meshes.new(name); me.from_pydata(verts, [], faces); me.materials.append(material)
    ob=bpy.data.objects.new(name, me); bpy.context.collection.objects.link(ob); bevel(ob,.035,3); smooth(ob); return ob


def cyl(name, radius, depth, loc, material, vertices=32, radial=None):
    bpy.ops.mesh.primitive_cylinder_add(vertices=vertices, radius=radius, depth=depth, location=loc)
    ob=bpy.context.object; ob.name=name; ob.data.materials.append(material); bevel(ob, min(radius*.18,.045), 2); smooth(ob)
    if radial is not None: ob.rotation_mode='QUATERNION'; ob.rotation_quaternion=Vector((0,0,1)).rotation_difference(Vector(radial).normalized())
    return ob


def make_cage(variant):
    configs = {
      'rust': ((.105,.115,.12), (.38,.105,.025), .70, (.36,.39,.39)),
      'black': ((.018,.020,.022), (.30,.32,.33), .18, (.46,.28,.055)),
      'zinc': ((.40,.43,.43), (.69,.71,.68), .20, (.055,.065,.075)),
      'enamel': ((.62,.085,.025), (.93,.28,.055), .22, (.35,.37,.37)),
    }
    base, accent, weather, boltcol=configs[variant]
    cage = metal(variant+'_cage', base, .37 if variant!='black' else .28, accent, weather, variant=='enamel')
    bolt = metal(variant+'_bolts', boltcol, .25, tuple(min(1,x*1.45+.08) for x in boltcol), .10)
    R=1.5; rodR=.095; ztop=1.56; zbot=-1.38
    # Eight bars are deliberately inside the strap rings.
    for i in range(8):
        a=2*math.pi*i/8; cyl('vertical rod',rodR,ztop-zbot,(1.34*math.cos(a),1.34*math.sin(a),(ztop+zbot)/2),cage)
    # top ring, three middle straps, and a notably heavier base band
    rings=[(1.48,.16),(.73,.14),(-.02,.14),(-.77,.14)]
    for z,h in rings: annular_band('flat strap ring',z,h,1.57,1.37,cage)
    annular_band('heavy base band',-1.34,.60,1.62,1.34,cage)
    # Rounded lip expresses the open top, without a lid.
    bpy.ops.mesh.primitive_torus_add(major_radius=1.48, minor_radius=.105, major_segments=96, minor_segments=16, location=(0,0,1.57))
    rim=bpy.context.object; rim.name='rolled top rim'; rim.data.materials.append(cage); smooth(rim)
    # Small weld beads at all strap crossings, including base top visual join.
    for z,_ in rings + [(-1.07,.1)]:
        for i in range(8):
            a=2*math.pi*i/8; bpy.ops.mesh.primitive_uv_sphere_add(segments=16, ring_count=8, radius=.125, location=(1.43*math.cos(a),1.43*math.sin(a),z))
            w=bpy.context.object; w.name='weld bead'; w.scale=(1,1,.55); w.data.materials.append(cage); smooth(w)
    # Twelve hex bolts and wider washers read clearly around the base.
    for i in range(12):
        a=2*math.pi*i/12; d=(math.cos(a),math.sin(a),0); p=Vector(d)
        cyl('washer', .145, .055, p*1.65+Vector((0,0,-1.34)), bolt, 36, d)
        cyl('hex bolt head', .115, .14, p*1.76+Vector((0,0,-1.34)), bolt, 6, d)


def point(obj, target): obj.rotation_euler=(Vector(target)-obj.location).to_track_quat('-Z','Y').to_euler()


def setup_scene(variant):
    bpy.ops.object.select_all(action='SELECT'); bpy.ops.object.delete(use_global=False)
    for d in (bpy.data.materials, bpy.data.meshes, bpy.data.curves, bpy.data.cameras, bpy.data.lights):
        pass
    sc=bpy.context.scene
    # Required final engine: Cycles CPU, four threads.
    sc.render.engine='CYCLES'; sc.cycles.device='CPU'; sc.render.threads_mode='FIXED'; sc.render.threads=4
    # This distribution was built without OpenImageDenoiser; clean output is
    # obtained with 128 CPU samples (the compositor denoiser is unavailable too).
    sc.cycles.samples=128; sc.cycles.use_denoising=False
    sc.render.resolution_x=sc.render.resolution_y=512; sc.render.resolution_percentage=100
    sc.render.image_settings.file_format='PNG'; sc.render.image_settings.color_mode='RGBA'; sc.render.film_transparent=True
    sc.view_settings.look='AgX - Medium High Contrast'
    sc.world.use_nodes=True
    bg=sc.world.node_tree.nodes.get('Background')
    bg.inputs['Color'].default_value=(.09,.11,.15,1); bg.inputs['Strength'].default_value=.32
    make_cage(variant)
    bpy.ops.object.camera_add(location=(4.45,-4.85,3.5)); cam=bpy.context.object; cam.data.lens=50; point(cam,(0,0,.0)); sc.camera=cam
    def area(name, loc, energy, color, size):
        bpy.ops.object.light_add(type='AREA', location=loc); o=bpy.context.object; o.name=name; o.data.energy=energy; o.data.color=color; o.data.shape='DISK'; o.data.size=size; point(o,(0,0,0)); return o
    area('warm key',(-4,-4,6),1800,(1.0,.53,.25),4.2)
    area('cool rim',(4,3.5,5),2100,(.25,.48,1.0),3.2)
    area('soft fill',(0,-4,1),850,(.55,.67,1.0),5.0)


def render(variant):
    os.makedirs(OUT, exist_ok=True); setup_scene(variant)
    path=os.path.join(OUT, 'cage_%s_512.png'%variant); bpy.context.scene.render.filepath=path
    bpy.ops.render.render(write_still=True)
    return path


if __name__ == '__main__':
    t=time.time(); opt=args(); out=render(opt.variant)
    print('Rendered %s in %.1fs' % (out, time.time()-t))
