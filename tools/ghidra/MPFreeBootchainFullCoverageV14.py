# @runtime PyGhidra
# @category MPFree
"""V14 exhaustive saved-project function graph, static data/xrefs and bounded decompilation.

READ-ONLY SHA-pinned owner's firmware. Executes no vendor code or hardware.
Does NOT claim runtime boot behavior or proof of silicon MaskROM.
Private Ghidra report MUST NOT be committed or shared publicly.
"""
import hashlib
import json
import os
import re
import time
from collections import defaultdict, deque
from datetime import datetime, timezone
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte

IMAGES = {
 "p1": (0xff704000, 32768, "6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
 "p3": (0x00100000, 363649, "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"),
}
ROOTS = {
 "p1": {
   "sram_rom_boot": [0xff704004,0xff704060,0xff7041a8,0xff705208],
   "storage_media": [0xff705310,0xff705350,0xff705398,0xff7054a4],
   "image_and_handoff": [0xff7052c0,0xff705308,0xff705598,0xff7055a4],
   "reset_and_recovery": [0xff705316,0xff7056a8,0xff708b74,0xff708cdc],
   "ddr_security": [0xff704164,0xff707f48,0xff7082c0],
 },
 "p3": {
   "fastboot_authorization": [0x00122780,0x00122834,0x00122854,0x00122980,0x00122bf8,0x00122c5c],
   "fastboot_ram": [0x001226b4,0x00122964,0x00122a10,0x0012247c],
   "image_boot_crc": [0x00101744,0x001017fc,0x0010a6a4,0x0010e000,0x0010e390,0x0010f388],
   "mmc_sd_boot": [0x001011e0,0x00106000,0x001095f8,0x00110484],
   "reboot_keys_pmu": [0x00100fe4,0x0010107e,0x001010a0,0x00101154,0x00112b20],
   "storage_recovery": [0x0011622a,0x0010b74c,0x0010b824],
 },
}
IMPORTANT_WORDS = ("fastboot","permission","oem","bootm","bootz","fdt","fit","uimage",
                   "mmc","sdmmc","boot_targets","bootcount","loader","bootrom",
                   "maskrom","usb","recovery","crc32","sha256","signature",
                   "gpio","adc","bootcmd","extlinux","saveenv","reboot")
args=list(getScriptArgs())
if len(args)!=2 or str(args[0]) not in IMAGES:
    raise RuntimeError("Usage: p1|p3 /absolute/report.json")
kind=str(args[0]); dest=Path(str(args[1]))
if not dest.is_absolute(): raise RuntimeError("Report path must be absolute")
base,size,expected=IMAGES[kind]
program=currentProgram
if program is None or str(program.getLanguageID())!="ARM:LE:32:v7":
    raise RuntimeError("Unexpected saved program architecture")
mem=program.getMemory()
space=program.getAddressFactory().getDefaultAddressSpace()
def A(n): return space.getAddress(int(n))
def H(n): return "0x%08x"%int(n)
block=mem.getBlock(A(base))
if block is None or block.getStart().getOffset()!=base or block.getSize()!=size:
    raise RuntimeError("Unexpected owner firmware block")
buf=JArray(JByte)(32768); raw=bytearray()
for offset in range(0,size,32768):
    count=min(32768,size-offset)
    if int(mem.getBytes(A(base+offset),buf,0,count))!=count:
        raise RuntimeError("Saved firmware block truncated")
    raw.extend(int(x)&255 for x in buf[:count])
raw=bytes(raw)
if hashlib.sha256(raw).hexdigest()!=expected:
    raise RuntimeError("Owner firmware image SHA256 mismatch")
listing=program.getListing(); fm=program.getFunctionManager(); refs=program.getReferenceManager()
functions=list(fm.getFunctions(True))
entries={}
outgoing=defaultdict(set); incoming=defaultdict(set)
indirect=[]
time_started=time.monotonic()
for fn in functions:
    start=int(fn.getEntryPoint().getOffset())
    item={"entry":H(start),"name":str(fn.getName()),
          "body_bytes":int(fn.getBody().getNumAddresses()),
          "calls":[],"indirect_sites":[],"instruction_count":0}
    iterator=listing.getInstructions(fn.getBody(),True)
    while iterator.hasNext():
        ins=iterator.next()
        item["instruction_count"]+=1
        ft=ins.getFlowType()
        if not ft.isCall() and not ft.isJump():continue
        address=int(ins.getAddress().getOffset())
        flows=[int(t.getOffset()) for t in ins.getFlows()]
        candidates=[]
        for target in flows:
            destfn=fm.getFunctionContaining(A(target))
            if destfn and ft.isCall():
                dst=int(destfn.getEntryPoint().getOffset())
                outgoing[start].add(dst);incoming[dst].add(start)
                candidates.append(H(dst))
        if ft.isCall():
            item["calls"].append({"site":H(address),"target_entries":candidates[:12],
                                  "flow_type":str(ft)})
            if not candidates:
                item["indirect_sites"].append({"address":H(address),"asm":str(ins)[:190]})
                indirect.append({"function":H(start),"at":H(address),"assembly":str(ins)[:190]})
    entries[start]=item

# Traverse both directions with explicit depth limits and record real graph paths.
roots={}
for domain, addresses in ROOTS[kind].items():
    resolved=set()
    for a in addresses:
        fn=fm.getFunctionContaining(A(a))
        if fn: resolved.add(int(fn.getEntryPoint().getOffset()))
    roots[domain]=sorted(resolved)
def breadth(seeds, adjacency, max_depth=4, max_nodes=800):
    seen={s:0 for s in seeds}; todo=deque((s,0) for s in seeds)
    while todo and len(seen)<max_nodes:
        node,depth=todo.popleft()
        if depth>=max_depth:continue
        for nxt in adjacency.get(node,()):
            if nxt not in seen:
                seen[nxt]=depth+1;todo.append((nxt,depth+1))
    return seen
regions={}
priority={}
for domain,seeds in roots.items():
    callers=breadth(seeds,incoming,4)
    callees=breadth(seeds,outgoing,4)
    regions[domain]={"root_functions":[H(x) for x in seeds],
                     "caller_reach_count":len(callers),"callee_reach_count":len(callees),
                     "callers":[{"function":H(k),"distance":v} for k,v in sorted(callers.items())],
                     "callees":[{"function":H(k),"distance":v} for k,v in sorted(callees.items())]}
    for a,v in list(callers.items())+list(callees.items()):
        priority[a]=min(priority.get(a,100),v)
for a in roots.values():
    for addr in a:priority[addr]=0

# Decompiled entire Ghidra-known function inventory, not just preselected addresses.
# Time limits apply PER FUNCTION, NOT to the run. Record every failure explicitly.
dec=DecompInterface();dec.toggleCCode(True);dec.setSimplificationStyle("decompile")
if not dec.openProgram(program):raise RuntimeError("Cannot initialize decompiler")
decompiled=0;failures=[];literal_hits=defaultdict(list);counts_by_domain=defaultdict(int)
try:
    order=sorted(entries, key=lambda a:(priority.get(a,100),a))
    for i,start in enumerate(order):
        item=entries[start]
        fn=fm.getFunctionAt(A(start))
        t=time.monotonic()
        try:
            result=dec.decompileFunction(fn,20,monitor)
            item["decompile_elapsed_s"]=round(time.monotonic()-t,3)
            if result and result.decompileCompleted() and result.getDecompiledFunction():
                code=str(result.getDecompiledFunction().getC())
                # Mask vendor authentication material and user/device identifiers.
                code=re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+",
                            "inmusic-unlock-magic-[REDACTED]",code)
                code=re.sub(r'(?i)"[^"\n]*(?:password|secret|private.key|serialno|ethaddr)[^"\n]*"',
                            '"[REDACTED]"',code)
                decompiled+=1
                hits=[w for w in IMPORTANT_WORDS if w.lower() in code.lower()]
                item["topic_words"]=hits
                for w in hits:literal_hits[w].append(H(start))
                # Full pseudocode for connected root neighborhoods; bounded excerpt otherwise.
                cap=18000 if priority.get(start,100)<=2 else 3500
                item["pseudocode"]=code[:cap]
                item["pseudocode_bytes"]=len(code)
                item["pseudocode_truncated"]=len(code)>cap
                item["decompile_status"]="OK"
            else:
                msg=str(result.getErrorMessage()) if result else "No decompilation"
                item["decompile_status"]="FAILED"
                item["decompile_error"]=msg[:320]
                failures.append({"address":H(start),"error":msg[:200]})
        except Exception as exc:
            item["decompile_elapsed_s"]=round(time.monotonic()-t,3)
            item["decompile_status"]="EXCEPTION"
            item["decompile_error"]=str(exc)[:320]
            failures.append({"address":H(start),"error":str(exc)[:200]})
finally:
    dec.dispose()
# Add bounded cross-references to critical literals and global pointer cells.
WATCH={
 "p3":{
   "fastboot_permission_global":0x0014d690,
   "pmu_reboot_register":0xff730094,
   "candidate_bootcount_register":0xff7300a0,
   "download_buffer_global":0x0014d684,
   "download_size_global":0x0014d680,
   "fastboot_dispatch_table":0x0012f2b0,
   "vendor_auth_text":0x00132bab,
 },
 "p1":{
   "pmu_reboot_register":0xff730094,
   "candidate_bootcount_register":0xff7300a0,
   "spl_selector_global":0xff70530c,
 }
}
xref_table={}
for name,addr in WATCH[kind].items():
    matches=[]
    try:
        for ref in refs.getReferencesTo(A(addr)):
            site=int(ref.getFromAddress().getOffset())
            owner=fm.getFunctionContaining(A(site))
            matches.append({"site":H(site),
                            "function":H(owner.getEntryPoint().getOffset()) if owner else None,
                            "type":str(ref.getReferenceType())})
            if len(matches)>=240:break
    except Exception as exc:
        matches=[{"error":str(exc)[:200]}]
    xref_table[name]={"address":H(addr),"sources":matches}
# Summarize raw literal matches, but don't infer execution from them.
literal_addresses={}
for name,addr in WATCH[kind].items():
    needle=int(addr).to_bytes(4,"little")
    offsets=[];pos=0
    while True:
        pos=raw.find(needle,pos)
        if pos<0:break
        offsets.append(H(base+pos))
        pos+=1
        if len(offsets)>=240:break
    literal_addresses[name]=offsets
report={
 "schema":"mpfree-v14-full-coverage-ghidra-v1",
 "source":kind,"date_utc":datetime.now(timezone.utc).isoformat(),
 "owner_sha256":expected,"owner_hash_verified":True,
 "hardware_contact":False,"vendor_code_executed":False,
 "statistics":{"ghidra_functions_total":len(functions),
               "functions_scanned":len(entries),
               "functions_decompiled":decompiled,
               "decompilation_failures":len(failures),
               "unique_direct_call_edges":sum(len(v) for v in outgoing.values()),
               "indirect_or_unresolved_call_sites":len(indirect),
               "elapsed_s":round(time.monotonic()-time_started,2),
               "function_coverage_ratio":decompiled/max(len(entries),1)},
 "domains":regions,"watched_global_references":xref_table,
 "raw_literal_sites":literal_addresses,
 "topic_index":dict(literal_hits),
 "indirect_call_sites":indirect,"failures":failures,
 "functions":{H(a):entries[a] for a in sorted(entries)},
 "limitations":[
   "Preexisting Ghidra project may fail to discover executable functions.",
   "Decompiling every recognized function does NOT emulate running vendor code.",
   "Function refs/indirect calls are incomplete without additional data-flow.",
   "A found string, address or call edge does not prove runtime reachability.",
   "Per-function pseudocode may be clipped; source binaries remain private.",
   "No physical MaskROM, USB recovery or eMMC restoration is proven."
 ]}
dest.parent.mkdir(parents=True,exist_ok=True)
old_umask=os.umask(0o077)
try:dest.write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")
finally:os.umask(old_umask)
print("V14 FULL-COVERAGE",kind,
      "functions",len(functions),"decompiled",decompiled,
      "unique_edges",report["statistics"]["unique_direct_call_edges"],
      "indirect",len(indirect),"elapsed",report["statistics"]["elapsed_s"])
