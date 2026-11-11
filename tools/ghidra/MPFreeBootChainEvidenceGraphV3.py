# @runtime PyGhidra
# @category MPFree
# Third-pass, whole-program evidence graph for the MPC One RK3288 SPL/U-Boot.
# Only -readOnly -noanalysis against already validated owner Ghidra projects.
# No MPC, USB, SSH, flash, ROM entry, or arbitrary target code execution.
import hashlib
import json
import os
import re
from collections import defaultdict, deque
from datetime import datetime, timezone
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte

EXPECTED = {
 "p1":(0xff704000,32768,"6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
 "p3":(0x00100000,363649,"b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"),
}
# Every address is an observed executable candidate, NOT an assertion
# that the routine is used in the live boot flow.
GROUPS={
 "p1":{
  "bootrom_sram_entry":[0xff704004,0xff704060,0xff7040b8,0xff705208,0xff705260],
  "source_selection":[0xff705350,0xff704120,0xff705342,0xff708f98,0xff705480],
  "uimage_loading_and_handoff":[0xff7052c0,0xff70529c,0xff705310,0xff7054a4,0xff705418],
  "ddr_stack_and_peripherals":[0xff7055b0,0xff7055bc,0xff705444,0xff704164,0xff704204,0xff707f48],
  "boot_failure_and_recovery":[0xff705316,0xff708b74,0xff708cdc,0xff7056a8,0xff708ae4],
  "mmc_driver_and_raw_reader":[0xff7072d8,0xff707b9c,0xff7069a0,0xff7082c0,0xff706798],
 },
 "p3":{
  "reset_mode_and_maskrom_hypotheses":[0x100de0,0x100edc,0x100fe4,0x101170,0x11622a,0x116216],
  "recovery_adc_gpio_pcb_revision":[0x1011e0,0x105620,0x1010a0,0x1147f0,0x115ef4,0x115f30],
  "environment_defaults_crc_fallback":[0x108804,0x108560,0x1085d4,0x1095f8,0x109b68,0x117eec],
  "boot_targets_sd_and_usb_drivers":[0x106000,0x106700,0x105958,0x116c8c,0x116f64,0x111d8c],
  "kernel_bootm_bootz_fit":[0x101744,0x1017fc,0x1018fc,0x10a6a4,0x10e390,0x10f728,0x127a1c],
  "fastboot_download_authorization":[0x122780,0x122834,0x122854,0x122980,0x122a10,0x12247c],
  "fastboot_ram_boot_handoff":[0x1226b4,0x1226b0,0x122964,0x122948,0x10d14c,0x11622a],
  "fastboot_flash_erase_gpt_recovery":[0x122bf8,0x122c5c,0x122cf8,0x122d38,0x116b70,0x116ec8],
  "memory_layout_and_relocation":[0x100180,0x1006e4,0x100768,0x1013e8,0x10b74c,0x10b824],
  "security_fit_hash_and_efuse":[0x1017fc,0x10e000,0x10e02a,0x10f388,0x10f728,0x127d5c],
  "usb_otg_serial_storage":[0x120654,0x120678,0x11c3ac,0x11cd54,0x11f580,0x116f64],
 },
}
WATCH={
 "p1":[0xff705274,0xff705268,0xff70530c,0xff7053fc,0xff70540c,
       0xff705598,0xff7055a4,0xff705308,0xff7041e2,0xff7091f8],
 "p3":[0xff730094,0xff770320,0xffb10000,0xffb40000,0xff580000,
       0x14d690,0x14d680,0x14d684,0x14d658,0x143e34,
       0x101154,0x122928,0x122ac4,0x122ad4,0x132cad,
       0x133006,0x133491,0x13363f],
}
STRING_CATEGORIES={
 "rom_and_reboot":["maskrom","brom","bootrom","reset_bootloader","reboot-bootloader",
                    "boot-mode","rockusb","reset to fastboot"],
 "boot_media":["mmc","sdmmc","emmc","usb start","boot_targets","scan_dev","extlinux",
               "boot.scr","sysboot","bootcmd"],
 "memory_and_handoff":["bootm","bootz","booti","loadaddr","ramdisk","fdt_addr",
                       "kernel_addr","bootm_low","bootm_size","memory","dram"],
 "fastboot":["fastboot","download","flashing","partition-size","partition-type"],
 "security":["efuse","otp","rsa","secure boot","sha256","signature","verified",
              "bad hash","checksum"],
 "board_identity":["recovery button","saradc","adc","gpio-keys","pcb rev","board_name",
                   "fdtfile","usb_reset"],
}
args=list(getScriptArgs())
if len(args)!=2 or args[0] not in EXPECTED: raise RuntimeError("Usage: p1|p3 /absolute/private.json")
kind=str(args[0]); dest=Path(str(args[1]))
if not dest.is_absolute(): raise RuntimeError("Report must be absolute")
base,length,sha=EXPECTED[kind]
p=currentProgram
if p is None or str(p.getLanguageID())!="ARM:LE:32:v7":raise RuntimeError("Expected ARM LE 32")
mem=p.getMemory(); space=p.getAddressFactory().getDefaultAddressSpace()
A=lambda n:space.getAddress(int(n))
H=lambda n:"0x%08x"%int(n)
block=mem.getBlock(A(base))
if block is None or block.getStart().getOffset()!=base or block.getSize()!=length:
    raise RuntimeError("Wrong Ghidra memory block")
b=JArray(JByte)(32768); raw=bytearray()
for off in range(0,length,32768):
    n=min(32768,length-off)
    if int(mem.getBytes(A(base+off),b,0,n))!=n:raise RuntimeError("Truncated Ghidra block")
    raw.extend(int(x)&255 for x in b[:n])
if hashlib.sha256(raw).hexdigest()!=sha:raise RuntimeError("Wrong vendor image SHA")
listing=p.getListing(); fm=p.getFunctionManager(); refs=p.getReferenceManager()
def incoming(v,cap=90):
    found=[]
    try:
        for r in refs.getReferencesTo(A(v)):
            src=r.getFromAddress()
            f=fm.getFunctionContaining(src)
            found.append({"at":str(src),"kind":str(r.getReferenceType()),
                          "caller":str(f.getEntryPoint()) if f else None})
            if len(found)>=cap:break
    except Exception as e:found.append({"error":str(e)})
    return found
def window(v,half=24,cap=60):
    if not (base<=v<base+length):return []
    start=max(v-half,base);end=min(v+half+100,base+length)
    inst=listing.getInstructionContaining(A(start))
    if inst is None:inst=listing.getInstructionAfter(A(start))
    ret=[]
    while inst and inst.getAddress().getOffset()<end and len(ret)<cap:
        ret.append({"at":str(inst.getAddress()),"instruction":str(inst),
                    "flows":[str(t) for t in inst.getFlows()]})
        inst=inst.getNext()
    return ret
def word(v):
    off=v-base
    if 0<=off<=length-4:
        return H(int.from_bytes(raw[off:off+4],"little"))
    return None
# Complete direct call graph for the *recognized* functions. The graph
# distinguishes calls from computed calls, jumps and data references.
graph={}
src_to_group=defaultdict(set)
known={}
functions=fm.getFunctions(True)
while functions.hasNext():
    fn=functions.next()
    ea=int(fn.getEntryPoint().getOffset())
    if not (base<=ea<base+length):continue
    known[ea]=fn
    calls=[];computed=[];branches=[]
    it=listing.getInstructions(fn.getBody(),True)
    while it.hasNext():
        ins=it.next()
        typ=ins.getFlowType()
        if not typ.isCall() and not typ.isJump():continue
        targets=[int(t.getOffset()) for t in ins.getFlows()]
        if typ.isCall():
            if targets:
                calls.extend(targets)
            else:
                if len(computed)<50:computed.append(str(ins.getAddress()))
        else:
            # Keep only non-local jumps to avoid enormous control-flow output.
            for target in targets:
                if not fn.getBody().contains(A(target)):
                    branches.append(target)
    graph[H(ea)]={"name":str(fn.getName()),"size":int(fn.getBody().getNumAddresses()),
                  "calls":sorted(set(H(t) for t in calls)),
                  "computed_calls":computed,
                  "out_of_function_branches":sorted(set(H(t) for t in branches))}
# Prioritize all categories equally rather than all functions sorted by VA.
def func_at(v):
    try:
        fn=fm.getFunctionContaining(A(v))
        return fn if fn and int(fn.getEntryPoint().getOffset()) in known else None
    except Exception:return None
queues=defaultdict(list)
for group, vals in GROUPS[kind].items():
    for v in vals:
        fn=func_at(v)
        if fn:
            ea=int(fn.getEntryPoint().getOffset())
            if ea not in queues[group]:queues[group].append(ea)
            src_to_group[ea].add(group)
selected={}
# Every explicit anchor is included first.
for group, items in queues.items():
    for ea in items:selected[ea]=known[ea]
# One-hop relevant callers and callees per category, with caps.
group_frontiers={}
for group, items in queues.items():
    hits=[]
    for ea in items:
        f=known[ea]
        for ref in refs.getReferencesTo(f.getEntryPoint()):
            if ref.getReferenceType().isCall():
                caller=func_at(ref.getFromAddress().getOffset())
                if caller: hits.append(int(caller.getEntryPoint().getOffset()))
        for callee in graph.get(H(ea),{}).get("calls",[]):
            f2=func_at(int(callee,16))
            if f2:hits.append(int(f2.getEntryPoint().getOffset()))
    uniq=list(dict.fromkeys(hits))
    group_frontiers[group]=uniq[:35]
    for ea in group_frontiers[group]:
        src_to_group[ea].add(group)
        selected[ea]=known[ea]
# Also collect functions referencing meaningful strings. Identify exact
# byte offsets; Ghidra xrefs might be missing for packed FDT string tables.
string_leads=defaultdict(list)
for m in re.finditer(rb"[\x20-\x7e]{5,}",bytes(raw)):
    content=m.group().decode("ascii","replace")
    for group, terms in STRING_CATEGORIES.items():
        if not any(t in content.lower() for t in terms):continue
        if re.search(r"unlock|secret|private|password|serialno|ethaddr|token|credential",content,re.I):
            content="[SENSITIVE BOOT STRING REDACTED]"
        va=base+m.start()
        matches=incoming(va,20)
        string_leads[group].append({"at":H(va),"text":content[:140],"xrefs":matches})
        for ref in matches:
            if ref.get("caller"):
                ea=int(ref["caller"],16)
                if ea in known and len(queues.get("strings_"+group,[]))<15:
                    queues["strings_"+group].append(ea)
                    selected[ea]=known[ea]
    if sum(len(x) for x in string_leads.values())>1500:break
# Data sinks and sources around permission/recovery/memory globals.
watches={}
for value in WATCH[kind]:
    rr=incoming(value,180)
    watches[H(value)]={
        "address":H(value),"raw_file_word":word(value),
        "raw_word_is_runtime_value":False,
        "references":rr,"local_instructions":window(value,48) if base<=value<base+length else [],
    }
    for ref in rr:
        if ref.get("caller"):
            ea=int(ref["caller"],16)
            if ea in known:
                selected[ea]=known[ea]
# Decompile selected function graph; bounded to avoid runaway analyses.
decompiler=DecompInterface()
decompiler.toggleCCode(True)
decompiler.setSimplificationStyle("decompile")
if not decompiler.openProgram(p):raise RuntimeError("Decompiler failed")
detailed={}
try:
    for ea in list(selected)[:240]:
        f=selected[ea]
        entry={"address":H(ea),"name":str(f.getName()),
               "categories":sorted(src_to_group.get(ea,set())),
               "incoming":incoming(ea,50),"summary":graph.get(H(ea),{})}
        try:
            dr=decompiler.decompileFunction(f,70,monitor)
            if dr and dr.decompileCompleted() and dr.getDecompiledFunction():
                code=str(dr.getDecompiledFunction().getC())
                code=re.sub(r'"[^"\n]{40,}"','"[LONG STRING REDACTED]"',code)
                code=re.sub(r'"[^"\n]*(?:password|unlock|secret|token|serialno|ethaddr)[^"\n]*"',
                            '"[PRIVATE STRING REDACTED]"',code,flags=re.I)
                entry["pseudocode"]=code[:18000]
                entry["truncated"]=len(code)>18000
            else:
                entry["error"]=str(dr.getErrorMessage()) if dr else "No result"
        except Exception as e:entry["error"]=str(e)
        detailed[H(ea)]=entry
finally:
    decompiler.dispose()
# Literal candidate search is NOT a code-flow proof. Includes 0xffb40000,
# the board's *nonsecure* eFuse block, distinct from 0xffb10000.
MAGICS={
 "rockchip_brom_request":0xef08a53c,"pmu_normal":0x5242c300,
 "pmu_loader":0x5242c301,"pmu_recovery":0x5242c303,
 "pmu_bootloader":0x5242c309,"pmu_reboot_register":0xff730094,
 "rk3288_secure_efuse":0xffb10000,"rk3288_nonsecure_efuse":0xffb40000,
 "rk3288_otg":0xff580000,"ram_fastboot_base":0x00800800,
 "uboot_image_load":0x00100000,"kernel_default":0x02000000,
}
literals={}
for label, value in MAGICS.items():
    positions=[]
    for endian in ("little","big"):
        needle=value.to_bytes(4,endian)
        off=raw.find(needle)
        while off>=0 and len(positions)<40:
            positions.append({"address":H(base+off),"endian":endian,
                              "aligned":off%4==0})
            off=raw.find(needle,off+1)
    literals[label]=positions
out={"schema":"mpfree-evidence-graph-v3","kind":kind,"date_utc":datetime.now(timezone.utc).isoformat(),
     "source":"static analysis, exact SHA validated, no live device",
     "sha256":sha,"image_base":H(base),"image_bytes":length,
     "all_known_function_graph":graph,
     "groups":{k:{"anchor_count":len(v),"anchors":[H(a) for a in v],
                  "extra_callers_callees":[H(a) for a in group_frontiers.get(k,[])]}
               for k,v in GROUPS[kind].items()},
     "detailed_functions":detailed,"watch_addresses":watches,
     "string_evidence":dict(string_leads),"literal_byte_hits":literals,
     "limitations":[
      "Direct code edges only. Indirect jump resolution requires manual validation.",
      "MMIO register descriptions are hypotheses unless supported by board DTB.",
      "Raw global bytes are not initialized runtime state, even when nonzero.",
      "MaskROM is not present in these files. No true ROM entry can be proved here.",
      "Authenticity and secure boot are not established by CRC32 or string hits.",
      "Private OEM reverse engineering: do not publish this report.",
     ]}
dest.parent.mkdir(parents=True,exist_ok=True)
umask=os.umask(0o077)
try:dest.write_text(json.dumps(out,indent=2)+"\n",encoding="utf-8")
finally:os.umask(umask)
print("MPFree evidence graph: kind",kind,"known functions",len(graph),
      "decompiled",len(detailed),"strings",sum(map(len,string_leads.values())),
      "report",str(dest))
