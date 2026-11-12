#!/usr/bin/env python3
"""V17 read-only owner Linux /boot and ACVA/ACVA-C DTB complete structural audit.

No original files are modified or mounted. debugfs reads a verified regular-file
ext4 clone. All reports derived; original kernel, device-tree and firmware
bytes remain outside the user ZIP.
"""
from __future__ import annotations
import argparse,hashlib,json,re,stat,struct,subprocess
from pathlib import Path
H=lambda b:hashlib.sha256(b).hexdigest()
FILES={
 "script":"/boot/boot.scr.uimg",
 "acva":"/boot/rk3288-az01-acva.dtb",
 "acvac":"/boot/rk3288-az01-acva-c.dtb",
 "kernel":"/boot/zImage-6.18.44-az01-2026-08-10-rt6",
}
def source(path):
    p=Path(path).expanduser()
    if (p.is_symlink() or not p.is_file() or not stat.S_ISREG(p.lstat().st_mode)
        or str(p).startswith(("/dev/","/proc/","/sys/"))):
        raise ValueError("Regular-file owner rootfs required")
    with p.open("rb") as f:
        f.seek(1080)
        if f.read(2)!=b"\x53\xef":raise ValueError("Expected ext4 source")
    return p
def read(image,path):
    proc=subprocess.run(["debugfs","-R","cat "+path,str(image)],
        capture_output=True,timeout=90,check=False)
    if proc.returncode or len(proc.stdout)<20 or len(proc.stdout)>32*1024*1024:
        raise RuntimeError("Unable to read expected owner boot data "+path+
                           " "+proc.stderr.decode(errors="replace")[:200])
    return proc.stdout
def parse_dtb(blob):
    if len(blob)<40:raise ValueError("FDT too short")
    h=struct.unpack_from(">10I",blob)
    magic,size,ofs,strings,resv,version,last,boot_cpu,string_size,struct_size=h
    if (magic!=0xd00dfeed or size>len(blob) or size<40 or
        ofs+struct_size>size or strings+string_size>size):
        raise ValueError("Invalid FDT header or bounds")
    names=blob[strings:strings+string_size]
    pos=ofs;stop=ofs+struct_size;stack=[];nodes={};node_order=[]
    def u32(off):return struct.unpack_from(">I",blob,off)[0]
    while pos+4<=stop:
        tag=u32(pos);pos+=4
        if tag==1:
            end=blob.find(b"\0",pos,stop)
            if end<0:raise ValueError("Unterminated FDT node")
            name=blob[pos:end].decode("utf-8","replace");pos=(end+4)&~3
            stack.append(name)
            path="/"+"/".join(x for x in stack if x)
            nodes.setdefault(path,{});node_order.append(path)
        elif tag==2:
            if not stack:raise ValueError("FDT node stack underflow")
            stack.pop()
        elif tag==3:
            if pos+8>stop:raise ValueError("FDT property bounds")
            nlen,noff=struct.unpack_from(">II",blob,pos);pos+=8
            if nlen>stop-pos or noff>=len(names):raise ValueError("FDT invalid property")
            nend=names.find(b"\0",noff)
            if nend<0:raise ValueError("FDT invalid property name")
            key=names[noff:nend].decode("utf-8","replace")
            value=blob[pos:pos+nlen];pos=(pos+nlen+3)&~3
            path="/"+"/".join(x for x in stack if x)
            if not stack:raise ValueError("FDT property outside node")
            scalar=value.endswith(b"\0") and all(32<=c<=126 or c==0 for c in value)
            if key in ("serial-number","mac-address","local-mac-address","bootargs","linux,initrd-start","linux,initrd-end"):
                display={"redacted":True,"sha256":H(value),"size":len(value)}
            elif scalar:
                display={"kind":"strings","value":[x for x in value.decode("ascii","replace").split("\0") if x]}
            elif len(value)%4==0 and len(value)<=96:
                display={"kind":"cells32","value":list(struct.unpack(">"+"I"*(len(value)//4),value))}
            else:
                display={"kind":"binary","size":len(value),"sha256":H(value),
                         "leading_hex":value[:32].hex()}
            nodes.setdefault(path,{})[key]=display
        elif tag==4:pass
        elif tag==9:break
        else:raise ValueError("Unexpected FDT token "+str(tag))
    return {"sha256":H(blob),"fdt_header":{
        "size":size,"version":version,"last_compatible":last,
        "struct_size":struct_size,"strings_size":string_size},
        "node_count":len(nodes),
        "property_count":sum(map(len,nodes.values())),
        "nodes":nodes,"node_order":node_order}
def difference(a,b):
    aa=a["nodes"];bb=b["nodes"];out=[]
    for node in sorted(set(aa)|set(bb)):
        if node not in aa or node not in bb:
            out.append({"node":node,"change":"only_in_acva" if node in aa else "only_in_acvac"})
            continue
        for key in sorted(set(aa[node])|set(bb[node])):
            if aa[node].get(key)!=bb[node].get(key):
                out.append({"node":node,"key":key,"acva":aa[node].get(key),
                            "acvac":bb[node].get(key)})
    return out
def script(blob):
    if len(blob)<64 or struct.unpack_from(">I",blob)[0]!=0x27051956:
        raise ValueError("Expected legacy U-Boot script header")
    from zlib import crc32
    hdr=bytearray(blob[:64]);expected=struct.unpack_from(">I",hdr,4)[0]
    hdr[4:8]=b"\0\0\0\0"
    count=struct.unpack_from(">I",blob,12)[0]
    data=blob[64:64+count]
    if len(data)!=count:raise ValueError("Truncated U-Boot script payload")
    # Historically U-Boot boot.scr payload is direct script text; some images
    # prepend a boot script count header. Keep both hypotheses explicit.
    decoded=data.decode("utf-8","replace")
    candidates=[decoded]
    if len(data)>8:
        n=int.from_bytes(data[:4],"big")
        if 0<n<=len(data)-8:candidates.append(data[8:8+n].decode("utf-8","replace"))
    chosen=max(candidates,key=lambda x:sum(w in x for w in ("bootz","bootm","load ","setenv","mmc","fdt")))
    lines=[line.strip() for line in chosen.splitlines() if line.strip() and
           not line.lstrip().startswith("#")]
    return {
        "sha256":H(blob),"header_crc_valid":(crc32(hdr)&0xffffffff)==expected,
        "payload_crc_valid":(crc32(data)&0xffffffff)==struct.unpack_from(">I",blob,24)[0],
        "script_commands":[re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+",
                                  "inmusic-unlock-magic-[REDACTED]",line) for line in lines],
        "candidate_decodings":len(candidates),"payload_bytes":len(data),
        "raw_boot_script_not_included":True}
def ranges(a):
    """Report only candidate overlaps, never claim physical DRAM safety."""
    anchors={"fastboot_rx":0x00800800,"fdt_default":0x01f00000,
             "kernel_default":0x02000000,"ramdisk_default":0x04000000}
    sizes=[0x10000,0x100000,0x400000,0x1000000,0x4000000,0x70000000]
    rows=[]
    for n in sizes:
        for name,address in anchors.items():
            if name=="fastboot_rx":continue
            rows.append({"assumed_download_size":n,"assumed_download_interval":
                         ["0x%08x"%anchors["fastboot_rx"],"0x%08x"%(anchors["fastboot_rx"]+n)],
                         "anchor":name,"anchor_address":"0x%08x"%address,
                         "would_cross_anchor":anchors["fastboot_rx"]<=address<
                                              anchors["fastboot_rx"]+n,
                         "proof_of_physical_overlap":False})
    return rows
def audit(rootfs):
    img=source(rootfs);payload={name:read(img,path) for name,path in FILES.items()}
    data={"schema":"mpfree-v17-owner-boot-complete-fdt","source_file":str(img),
          "source_size":img.stat().st_size,
          "source_sha256":H(img.read_bytes()) if img.stat().st_size<1024*1024*1024 else None,
          "read_only":True,"file_hashes":{n:H(v) for n,v in payload.items()}}
    a=parse_dtb(payload["acva"]);b=parse_dtb(payload["acvac"])
    data["acva"]=a;data["acva_c"]=b
    data["dtb_complete_differences"]=difference(a,b)
    data["script"]=script(payload["script"])
    data["kernel"]={
        "sha256":H(payload["kernel"]),"bytes":len(payload["kernel"]),
        "arm_zimage_magic":payload["kernel"][0x24:0x28]==b"\x18\x28\x6f\x01",
        "raw_kernel_not_included":True}
    data["candidate_memory_crossings"]=ranges(data)
    data["limitations"]=[
       "This parses actual owner DTBs and boot script, but does not emulate RK3288.",
       "Boot addresses are candidates until U-Boot relocations and DRAM map confirmed.",
       "No original binaries are included in reports or changed."
    ]
    return data
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("rootfs",type=Path);p.add_argument("report",type=Path)
    a=p.parse_args()
    r=audit(a.rootfs)
    if not a.report.is_absolute() or a.report.is_symlink():
        p.error("Must provide a new absolute private report path")
    a.report.parent.mkdir(parents=True,exist_ok=True)
    a.report.write_text(json.dumps(r,indent=2)+"\n")
    print("Complete owner DTB diff",len(r["dtb_complete_differences"]),
          "nodes",r["acva"]["node_count"],r["acva_c"]["node_count"],
          "script commands",len(r["script"]["script_commands"]))
if __name__=="__main__":main()
