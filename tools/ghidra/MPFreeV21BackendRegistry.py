# @runtime PyGhidra
# @category MPFree
"""MPFree V21: owner-P3 full value-flow audit of backend registration and
SD/MMC U-Boot command definitions. SHA-pinned, saved-project read-only.

Unlike V20's address-only STORE census, inspects STORED VALUES, input
dependency trees, candidate function pointers, the linked-list sentinel,
registration-site callers, and boot-command strings/xrefs. Emits complete
private pseudocode for the relevant owning functions and their neighbors.
Never executes U-Boot or touches any physical hardware.
"""
import hashlib
import json
import os
import re
import time
from collections import Counter,defaultdict,deque
from pathlib import Path
from datetime import datetime,timezone
from ghidra.app.decompiler import DecompInterface
from jpype import JArray,JByte

args=list(getScriptArgs())
if len(args)!=3 or any(not Path(str(x)).is_absolute() for x in args):
    raise RuntimeError("Expected /abs/report-dir /abs/private-P3 /abs/ready-file")
out=Path(str(args[0]));private=Path(str(args[1]));marker=Path(str(args[2]))
os.umask(0o077)
out.mkdir(parents=True,exist_ok=True)
BASE=0x00100000; SIZE=363649
SHA="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
program=currentProgram
if program is None or str(program.getLanguageID())!="ARM:LE:32:v7":
    raise RuntimeError("Saved owner P3 ARM project required")
space=program.getAddressFactory().getDefaultAddressSpace()
def A(x):return space.getAddress(int(x)&0xffffffff)
def H(x):return "0x%08x"%(int(x)&0xffffffff)
mem=program.getMemory();block=mem.getBlock(A(BASE))
if block is None or block.getStart().getOffset()!=BASE or block.getSize()!=SIZE:
    raise RuntimeError("Unexpected owner P3 memory image")
buf=JArray(JByte)(32768);raw=bytearray()
for offset in range(0,SIZE,32768):
    n=min(SIZE-offset,32768)
    if int(mem.getBytes(A(BASE+offset),buf,0,n))!=n:
        raise RuntimeError("Owner P3 image truncated in Ghidra")
    raw.extend(int(v)&255 for v in buf[:n])
raw=bytes(raw)
if hashlib.sha256(raw).hexdigest()!=SHA:raise RuntimeError("Wrong owner's P3 firmware hash")
if private.is_symlink() or marker.is_symlink():
    raise RuntimeError("Refusing private guest export through symlink")
private.parent.mkdir(parents=True,exist_ok=True)
with private.open("xb") as f:
    f.write(raw);f.flush();os.fsync(f.fileno())
if hashlib.sha256(private.read_bytes()).hexdigest()!=SHA:
    raise RuntimeError("Private P3 export verification failed")
marker.write_text(SHA+"\n")
print("V21 P3 owner SHA verified; isolated QEMU may start",flush=True)
def read32(at):
    if BASE<=at<=BASE+SIZE-4:
        return int.from_bytes(raw[at-BASE:at-BASE+4],"little")
    return None
fm=program.getFunctionManager();listing=program.getListing();refs=program.getReferenceManager()
fns={int(fn.getEntryPoint().getOffset()):fn for fn in fm.getFunctions(True)}
EDGES=defaultdict(set);BACK=defaultdict(set);SITES=defaultdict(list)
for entry,fn in fns.items():
    it=listing.getInstructions(fn.getBody(),True)
    while it.hasNext():
        ins=it.next()
        if not ins.getFlowType().isCall():continue
        for destination in ins.getFlows():
            fn2=fm.getFunctionContaining(destination)
            if fn2:
                dst=int(fn2.getEntryPoint().getOffset())
                if dst in fns:
                    EDGES[entry].add(dst);BACK[dst].add(entry)
                    SITES[(entry,dst)].append(H(ins.getAddress().getOffset()))
ROOTS={
 "backend_lookup":0x0011c3ac,
 "backend_projection":0x0011c3ec,
 "storage_update":0x0010b74c,
 "storage_read":0x0010b824,
 "backend_register_candidate":0x0011d048,
 "backend_init_candidate":0x00116f64,
 "backend_register_candidate_2":0x0011dcb0,
 "backend_related_init":0x0011c9e0,
 "backend_related_storage":0x0011d968,
 "backend_related_driver":0x0011de50,
 "command_wrapper":0x00110484,
 "command_parser":0x001095f8,
 "command_interpreter":0x00109b68,
 "command_dispatch":0x00109688,
 "boot_media_init":0x001011e0,
 "boot_script_load":0x00106000,
 "bootm":0x00101744,
 "fit_image_decode":0x0010e390,
 "oem_entry":0x00122854,
}
roots={}
for name,at in ROOTS.items():
    fn=fm.getFunctionContaining(A(at))
    if fn:
        roots[name]=int(fn.getEntryPoint().getOffset())
def graph_depth(start,graph):
    distance={start:0};q=deque([start])
    while q:
        node=q.popleft()
        for next_ in graph.get(node,()):
            if next_ not in distance:
                distance[next_]=distance[node]+1;q.append(next_)
    return distance
domains={}
priority={}
for name,addr in roots.items():
    callers=graph_depth(addr,BACK);callees=graph_depth(addr,EDGES)
    domains[name]={
        "root":H(addr),"caller_reach":len(callers),"callee_reach":len(callees),
        "nearest_callers":[{"entry":H(k),"distance":v}
                           for k,v in sorted(callers.items(),key=lambda t:(t[1],t[0]))[:160]],
        "nearest_callees":[{"entry":H(k),"distance":v}
                           for k,v in sorted(callees.items(),key=lambda t:(t[1],t[0]))[:160]]
    }
    for graph in (callers,callees):
        for entry,d in graph.items():
            priority[entry]=min(priority.get(entry,999999),d)

# The list sentinel lives at the address contained in owner literal cell
# 0x0011c3d4. Distinguish the literal, its pointed-to head and list records.
SENTINEL_CELL=0x0011c3d4
sentinel=read32(SENTINEL_CELL)
def refs_to(at,limit=250):
    values=[]
    if at is None:return values
    try:
        for ref in refs.getReferencesTo(A(at)):
            caller=fm.getFunctionContaining(ref.getFromAddress())
            values.append({"site":H(ref.getFromAddress().getOffset()),
                "caller":H(caller.getEntryPoint().getOffset()) if caller else None,
                "type":str(ref.getReferenceType())})
            if len(values)>=limit:break
    except Exception as exc:
        values=[{"error":str(exc)[:200]}]
    return values
def word_refs_to(at,limit=300):
    needle=int(at).to_bytes(4,"little")
    pos=0;hits=[]
    while len(hits)<limit:
        pos=raw.find(needle,pos)
        if pos<0:break
        hits.append(H(BASE+pos))
        pos+=1
    return hits
sentinel_evidence={
 "literal_cell":H(SENTINEL_CELL),
 "literal_value":H(sentinel) if sentinel is not None else None,
 "head_source_note":"Pointer literal loaded by FUN_0011c3ac; live RAM contents are not known",
 "literal_cell_ghidra_xrefs":refs_to(SENTINEL_CELL),
 "head_ghidra_xrefs":refs_to(sentinel),
 "head_pointer_literal_sites":word_refs_to(sentinel) if sentinel is not None else [],
 "expected_backend_next_field":H(0),
 "expected_backend_id_offset":H(0xcc),
 "projected_backend_offset":H(0xc8),
 "projected_method_offset":H(0x68),
 "effective_method_offset":H(0x130),
}

# Relevant owner boot command strings; do not emit any secret/token/serial
# data from the owner firmware, only exact whitelist matches and locations.
KEYWORDS=(b"boot_targets",b"bootcmd",b"distro_bootcmd",
          b"mmc dev",b"mmc list",b"bootz",b"bootm",b"ext4load",
          b"fatload",b"sdmmc",b"sdcard",b"rockusb",b"loader",b"recovery")
bootstrings=[]
for key in KEYWORDS:
    offset=0
    while True:
        pos=raw.lower().find(key,offset)
        if pos<0:break
        at=BASE+pos
        bootstrings.append({"whitelisted_token":key.decode("ascii"),"address":H(at),
                            "direct_xrefs":refs_to(at,32),
                            "raw_pointer_sites":word_refs_to(at,50)})
        offset=pos+1
bootstrings.sort(key=lambda x:(x["whitelisted_token"],x["address"]))
# Full expression tree with a strict node budget; constant/address facts
# never claim to identify a physical storage device.
def expression(v,depth=0,seen=None,budget=None):
    if budget is None:budget=[160]
    if budget[0]<=0:return {"kind":"budget-exhausted"}
    budget[0]-=1
    if v is None:return {"kind":"null"}
    if seen is None:seen=set()
    row={"varnode":str(v)[:130],"bytes":int(v.getSize())}
    if v.isConstant():
        row.update({"kind":"constant","value":H(v.getOffset())})
        return row
    if v.isAddress():
        location=int(v.getAddress().getOffset())
        row["storage_address"]=H(location)
        value=read32(location)
        if value is not None:row["static_owner_word_at_storage_address"]=H(value)
    op=v.getDef()
    if op is None:
        row["kind"]="unbound-input"
        return row
    identifier=(str(v),str(op.getSeqnum().getTarget()))
    if identifier in seen:
        row["kind"]="recursive-phi"
        return row
    row.update({"kind":str(op.getMnemonic()),
                "source_pc":str(op.getSeqnum().getTarget())})
    row["inputs"]=[
       expression(op.getInput(i),depth+1,seen|{identifier},budget)
       for i in range(min(op.getNumInputs(),7))]
    if op.getNumInputs()>7:row["truncated_inputs"]=op.getNumInputs()-7
    return row
def literal_constants(v,depth=0,seen=None,budget=None):
    if budget is None:budget=[80]
    if v is None or depth>=10 or budget[0]<=0:return set()
    budget[0]-=1
    if seen is None:seen=set()
    token=str(v)
    if token in seen:return set()
    seen=seen|{token}
    if v.isConstant():return {int(v.getOffset())&0xffffffff}
    op=v.getDef()
    if op is None:return set()
    vals=set()
    for j in range(min(op.getNumInputs(),7)):
        vals|=literal_constants(op.getInput(j),depth+1,seen,budget)
    return vals
def asm(at):
    ins=listing.getInstructionContaining(A(at))
    if not ins:return {"pc":H(at),"asm":None}
    return {"pc":H(at),"asm":str(ins)[:180],
            "flows":[str(p) for p in ins.getFlows()][:8]}
# Priorities: exact callback registration fields first, then full census.
TARGET_OFFSETS={0x68,0x6c,0xc8,0xcc,0x130,0x33,0x144}
HIGH_VALUE_FUNCS={
    0x0011d048,0x0011c3ac,0x0011c3ec,0x0010b74c,0x0010b824,
    0x00116f64,0x0011dcb0,0x0011c9e0,0x0011d968,0x0011de50,
    0x001095f8,0x00109688,0x00109b68,0x00110484,
    0x001011e0,0x00106000,0x00101744,0x00122854,
}
VALUE_STORES=out/"V21-P3-BACKEND-VALUE-WRITES.jsonl"
IMPORTANT_FUNCS=out/"V21-P3-BACKEND-AND-BOOT-FUNCTIONS.jsonl"
stats=Counter();failures=[];interesting=[]
dec=DecompInterface();dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(program):raise RuntimeError("Ghidra owner decompiler unavailable")
started=time.monotonic()
try:
    with VALUE_STORES.open("w") as dst, IMPORTANT_FUNCS.open("w") as detail:
        for index,entry in enumerate(sorted(fns,key=lambda x:(priority.get(x,999999),x))):
            fn=fns[entry]
            important=(entry in HIGH_VALUE_FUNCS or priority.get(entry,999999)<=1)
            summary_row={
              "entry":H(entry),"name":str(fn.getName()),
              "root_distance":priority.get(entry),
              "direct_callees":[H(x) for x in sorted(EDGES.get(entry,()))],
              "direct_callers":[H(x) for x in sorted(BACK.get(entry,()))],
              "store_value_witnesses":[],"indirect_witnesses":[],
              "failure":None
            }
            try:
                result=dec.decompileFunction(fn,30,monitor)
                if not result or not result.decompileCompleted() or not result.getHighFunction():
                    raise RuntimeError(str(result.getErrorMessage()) if result else "empty decompilation")
                stats["decompiled"]+=1
                code=str(result.getDecompiledFunction().getC())
                code=re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+",
                            "inmusic-unlock-magic-[REDACTED]",code)
                if important:summary_row["pseudocode"]=code[:75000]
                ops=result.getHighFunction().getPcodeOps()
                while ops.hasNext():
                    op=ops.next();kind=str(op.getMnemonic())
                    if kind not in ("STORE","LOAD","CALLIND","BRANCHIND"):continue
                    at=int(op.getSeqnum().getTarget().getOffset())
                    if kind=="STORE":
                        addr=op.getInput(1) if op.getNumInputs()>1 else None
                        value=op.getInput(2) if op.getNumInputs()>2 else None
                        offsets=literal_constants(addr)
                        vals=literal_constants(value)
                        candidate_pointers=sorted({
                            H(c&~1) for c in vals if (c&~1) in fns
                        })
                        is_relevant=bool(
                            important or offsets&TARGET_OFFSETS or
                            candidate_pointers or sentinel in offsets)
                        stats["all_store_sites"]+=1
                        if is_relevant:
                            row={
                              "fn":H(entry),"at":H(at),"asm":asm(at),
                              "offset_constants":[H(x) for x in sorted(offsets)[:30]],
                              "source_value_constants":[H(x) for x in sorted(vals)[:30]],
                              "known_function_pointer_literals":candidate_pointers[:35],
                              "address_dependencies":expression(addr),
                              "stored_value_dependencies":expression(value),
                              "owner_code_executed":False,
                              "physical_destination_verified":False
                            }
                            dst.write(json.dumps(row)+"\n")
                            stats["relevant_value_stores"]+=1
                            if candidate_pointers:stats["function_pointer_value_stores"]+=1
                            if offsets&TARGET_OFFSETS:stats["field_offset_stores"]+=1
                            if important:summary_row["store_value_witnesses"].append(row)
                    elif kind in ("CALLIND","BRANCHIND"):
                        stats["all_indirect_sites"]+=1
                        target=op.getInput(0) if op.getNumInputs()>0 else None
                        vals=literal_constants(target)
                        if important or vals:
                            row={"at":H(at),"asm":asm(at),
                                 "target_expr":expression(target),
                                 "literal_candidates":[H(x) for x in sorted(vals)[:20]],
                                 "actual_runtime_target_known":False}
                            summary_row["indirect_witnesses"].append(row)
                            stats["indirect_sites_traced"]+=1
                if (important or summary_row["store_value_witnesses"] or
                    summary_row["indirect_witnesses"]):
                    detail.write(json.dumps(summary_row)+"\n")
                    stats["detail_functions"]+=1
            except Exception as exc:
                failures.append({"entry":H(entry),"error":str(exc)[:600]})
                stats["decompile_failures"]+=1
            if (index+1)%100==0:
                dst.flush();detail.flush()
                print("V21 P3 registry/value-flow",index+1,"/",len(fns),
                      "STORE",stats["all_store_sites"],
                      "value-witness",stats["relevant_value_stores"],
                      "pointer stores",stats["function_pointer_value_stores"],
                      "indirect",stats["all_indirect_sites"],
                      "seconds",round(time.monotonic()-started,1))
finally:
    dec.dispose()
summary={
 "schema":"mpfree-v21-verified-backend-registry-and-boot",
 "timestamp_utc":datetime.now(timezone.utc).isoformat(),
 "owner_input_sha256":SHA,"owner_source_verified":True,
 "recognized_functions":len(fns),
 "decompiled_functions":stats["decompiled"],
 "direct_call_edges":sum(len(k) for k in EDGES.values()),
 "statistics":dict(stats),"decompile_failures":failures,
 "storage_list_registry":sentinel_evidence,
 "backend_and_boot_roots":domains,
 "boot_command_string_pointer_inventory":bootstrings,
 "owner_static_function_pointer_literal_sites":[
     {"target":H(value&~1),"at":H(BASE+off),"thumb":bool(value&1)}
     for off in range(0,len(raw)-4,4)
     for value in [int.from_bytes(raw[off:off+4],"little")]
     if (value&~1) in fns
 ][:3500],
 "full_value_store_report":VALUE_STORES.name,
 "deep_source_report":IMPORTANT_FUNCS.name,
 "elapsed_seconds":round(time.monotonic()-started,2),
 "caveats":[
    "An offset in a P-code dependency expression is not an identified physical address.",
    "Only the exact parenthesized expression and value dataflow can justify callback registration.",
    "A callback pointer in static code may differ from the device's runtime list contents.",
    "No real storage, MMC, Fastboot or bootm operation was executed by Ghidra.",
    "Saved project may omit code functions not discovered by Ghidra."
 ]
}
(out/"V21-P3-BACKEND-REGISTRY-SUMMARY.json").write_text(json.dumps(summary,indent=2)+"\n")
print("V21 source registry COMPLETE",stats["decompiled"],"functions;",
      stats["relevant_value_stores"],"STORE values;",
      stats["indirect_sites_traced"],"indirect expressions;",
      stats["decompile_failures"],"decompilation errors")
