"""Read PE resources without loading or executing the original mail runtime."""
import csv, hashlib, json, struct
from pathlib import Path

source = Path(r"C:\Program Files\Windows Mail\MSOERES.dll")
output = Path(__file__).resolve().parent.parent / "analysis" / "ui-resources"
output.mkdir(parents=True, exist_ok=True)
data = source.read_bytes()
u16 = lambda off: struct.unpack_from("<H", data, off)[0]
u32 = lambda off: struct.unpack_from("<I", data, off)[0]
pe = u32(0x3C)
optional = pe + 24
sections = []
for i in range(u16(pe + 6)):
    s = optional + u16(pe + 20) + i * 40
    sections.append((u32(s+12), max(u32(s+8),u32(s+16)),u32(s+20)))
def offset(rva):
    for start, size, raw in sections:
        if start <= rva < start + size: return raw + rva - start
    raise ValueError("RVA outside sections")
base = offset(u32(optional + (112 if u16(optional)==0x20B else 96) + 16))
resources = {}
def directory(at, names=()):
    count = u16(base+at+12) + u16(base+at+14)
    for i in range(count):
        name, target = struct.unpack_from("<II",data,base+at+16+i*8)
        if name & 0x80000000:
            n = base + (name & 0x7FFFFFFF)
            name = data[n+2:n+2+u16(n)*2].decode("utf-16le")
        keys = names+(name,)
        if target & 0x80000000: directory(target & 0x7FFFFFFF,keys)
        else:
            pos = base+target
            rva, length = struct.unpack_from("<II",data,pos)
            resources[keys] = data[offset(rva):offset(rva)+length]
directory(0)
rows=[]
for keys, raw in resources.items():
    if keys[0] == 2:
        header, width, height, planes, bits = struct.unpack_from("<IiiHH",raw)
        used = struct.unpack_from("<I",raw,32)[0] if header>=40 else 0
        palette = (used or (1<<bits if bits<=8 else 0))*4
        masks = 12 if header==40 and struct.unpack_from("<I",raw,16)[0]==3 else 0
        pixel = 14+header+palette+masks
        bmp = b"BM"+struct.pack("<IHHI",len(raw)+14,0,0,pixel)+raw
        path=output/f"bitmap-{keys[1]}-{keys[2]}.bmp"
        path.write_bytes(bmp)
        rows.append(dict(type="bitmap",id=keys[1],language=keys[2],width=width,height=height,bits=bits,file=path.name))
    elif keys[0] == 14:
        count=struct.unpack_from("<H",raw,4)[0]
        images=[]; entries=[]; position=6+16*count
        for i in range(count):
            record=raw[6+i*14:6+(i+1)*14]
            image_id=struct.unpack_from("<H",record,12)[0]
            image=resources.get((3,image_id,keys[2]))
            if image is None: image=next(value for key,value in resources.items() if key[0]==3 and key[1]==image_id)
            entries.append(record[:8]+struct.pack("<II",len(image),position));images.append(image);position+=len(image)
        path=output/f"icon-{keys[1]}-{keys[2]}.ico"
        path.write_bytes(struct.pack("<HHH",0,1,count)+b"".join(entries)+b"".join(images))
        rows.append(dict(type="icon",id=keys[1],language=keys[2],width="",height="",bits="",file=path.name))
with (output/"inventory.csv").open("w",encoding="utf-8",newline="") as file:
    writer=csv.DictWriter(file,fieldnames=["type","id","language","width","height","bits","file"]);writer.writeheader();writer.writerows(rows)
(output/"source.json").write_text(json.dumps({"source":str(source),"sha256":hashlib.sha256(data).hexdigest(),"resource_count":len(rows)},indent=2),encoding="utf-8")
print(json.dumps(rows,ensure_ascii=False))
