"""Author offline HDR studio cubemaps. KTX2 RGBA16F, linear, no external assets.

Specular mips use GGX importance sampling, diffuse uses cosine-weighted integration.
Python standard library only; also runs inside the connected Blender.
"""
import math
import struct
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'game/assets/lighting'
OUT.mkdir(parents=True,exist_ok=True)


def normalize(v):
    r=math.sqrt(sum(x*x for x in v))
    return tuple(x/r for x in v)


def cross(a,b):
    return (a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0])


def radiance(v):
    x,y,z=v
    # Dark teal surroundings, two large warm/cool photographic softboxes.
    sky=max(y,0)**0.7
    base=[0.08+0.25*sky,0.12+0.31*sky,0.17+0.4*sky]
    for direction,color,power,sharpness in [((-0.5,0.75,0.4),(1,0.85,0.63),8,25),((0.65,0.5,-0.55),(0.5,0.85,1),5,40),((0,0.95,-0.2),(1,1,1),2,8)]:
        d=normalize(direction)
        s=power*max(sum(a*b for a,b in zip(v,d)),0)**sharpness
        base=[a+s*b for a,b in zip(base,color)]
    return base


def direction(face,u,v):
    return normalize([(1,-v,-u),(-1,-v,u),(u,1,v),(u,-1,-v),(u,-v,1),(-u,-v,-1)][face])


def integrate(n,roughness,diffuse=False):
    if roughness==0 and not diffuse:
        return radiance(n)
    tangent=normalize(cross((0,1,0) if abs(n[1])<0.99 else (1,0,0),n))
    bitangent=cross(n,tangent)
    total=[0,0,0]
    weight=0
    for i in range(64):
        xi=(i+0.5)/64
        phi=i*2.399963229728653
        if diffuse:
            cos_t=math.sqrt(1-xi)
        else:
            alpha=roughness*roughness
            cos_t=math.sqrt((1-xi)/(1+(alpha*alpha-1)*xi))
        sin_t=math.sqrt(max(0,1-cos_t*cos_t))
        h=tuple(tangent[j]*math.cos(phi)*sin_t+bitangent[j]*math.sin(phi)*sin_t+n[j]*cos_t for j in range(3))
        if diffuse:
            ray,w=h,1
        else:
            ndh=sum(a*b for a,b in zip(n,h))
            ray=tuple(2*ndh*h[j]-n[j] for j in range(3))
            w=max(sum(a*b for a,b in zip(n,ray)),0)
        c=radiance(ray)
        total=[a+b*w for a,b in zip(total,c)]
        weight+=w
    return [a/max(weight,1e-8) for a in total]


def write_ktx(name,size,levels,diffuse):
    payload=[]
    for mip in range(levels):
        edge=max(1,size>>mip)
        roughness=mip/max(1,levels-1)
        data=bytearray()
        for face in range(6):
            for y in range(edge):
                for x in range(edge):
                    n=direction(face,2*(x+0.5)/edge-1,2*(y+0.5)/edge-1)
                    color=integrate(n,roughness,diffuse)
                    data.extend(struct.pack('<4e',*color,1))
        payload.append(bytes(data))
    # KHR_DF_MODEL_RGBSDA; BT709 primaries; linear transfer; 16-bit signed float channels.
    dfd=struct.pack('<IHHHH4B4B8B',92,0,0,2,88,1,1,1,0,0,0,0,0,8,0,0,0,0,0,0,0)
    for i,channel in enumerate([0,1,2,15]):
        dfd+=struct.pack('<HBB4BII',i*16,15,channel|0xC0,0,0,0,0,0,0x3F800000)
    assert len(dfd)==92
    dfd_offset=80+levels*24
    offset=(dfd_offset+len(dfd)+7)//8*8
    # 12-byte magic; 13 u32; 2 u64 = 80-byte header.
    header=b'\xABKTX 20\xBB\r\n\x1A\n'+struct.pack('<13I2Q',97,2,size,size,0,0,6,levels,0,dfd_offset,len(dfd),0,0,0,0)
    indices=bytearray()
    body=bytearray()
    for data in payload:
        indices.extend(struct.pack('<3Q',offset+len(body),len(data),len(data)))
        body.extend(data)
    blob=header+indices+dfd
    blob+=bytes(offset-len(blob))
    (OUT/name).write_bytes(blob+body)
    print(name,len(blob+body),'bytes')


write_ktx('studio_specular.ktx2',64,7,False)
write_ktx('studio_diffuse.ktx2',16,1,True)
