# @runtime PyGhidra
# @category MPFree
"""V20 P3 read-only backend-vtable / command interpreter / SD boot analysis.

For every recognized P3 function, decompile and inspect High P-code for
writes/reads of potential callback-table fields, indirect call sites,
registration/list-head paths, and parser dispatch. Preserve expression
dependencies, actual instruction witnesses, and unresolved operations.

THIS IS STATIC ARM ANALYSIS, not eMMC or boot execution.
"""
from ghidra.app.decompiler import DecompInterface
from jpype import JArray,JByte
from collections import Counter,defaultdict,deque
from datetime import datetime,timezone
from pathlib import Path
import hashlib,json,os,re,time

args=list(getScriptArgs())
if len(args)!=1 or not Path(str(args[0])).is_absolute():
    raise RuntimeError("Expected one absolute private output directory")
folder=Path(str(args[0]))
folder.mkdir(parents=True,exist_ok=True)
os.umask(0o077)
base=0x00100000
length=363649
expected="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
P=currentProgram
if P is None or str(P.getLanguageID())!="ARM:LE:32:v7":
    raise RuntimeError("Incorrect saved owner P3 architecture")
space=P.getAddressFactory().getDefaultAddressSpace()
def A(x):return space.getAddress(int(x)&0xffffffff)
def H(x):return "0x%08x"%(int(x)&0xffffffff)
mem=P.getMemory()
block=mem.getBlock(A(base))
if block is None or block.getStart().getOffset()!=base or block.getSize()!=length:
    raise RuntimeError("Unexpected owner P3 source block")
buf=JArray(JByte)(32768);raw=bytearray()
for off in range(0,length,32768):
    n=min(32768,length-off)
    if int(mem.getBytes(A(base+off),buf,0,n))!=n:
        raise RuntimeError("Owner P3 read failed")
    raw.extend(int(x)&255 for x in buf[:n])
raw=bytes(raw)
if hashlib.sha256(raw).hexdigest()!=expected:
    raise RuntimeError("Owner P3 SHA256 mismatch")
fm=P.getFunctionManager()
listing=P.getListing()
refs=P.getReferenceManager()
funcs={int(f.getEntryPoint().getOffset()):f for f in fm.getFunctions(True)}
KEY={
  "persist_entry":0x0010b74c,
  "backend_lookup":0x0011c3ac,
  "backend_object":0x0011c3ec,
  "backend_status":0x0011daf4,
  "blob_prepare":0x0010cdf4,
  "blob_extent":0x0010b728,
  "backend_alignment":0x0012ee08,
  "command_wrapper":0x00110484,
  "command_parser":0x001095f8,
  "command_dispatch":0x00109b68,
  "command_allocator":0x00108de0,
  "command_free":0x0010d588,
  "board_boot_policy":0x00100fe4,
  "mmc_probe":0x001011e0,
  "load_source":0x00106000,
  "bootm":0x00101744,
  "bootm_image":0x0010a6a4,
  "oem_entry":0x00122854,
  "guest_download":0x00122980
}
ROOT_OFFSETS={0x130,0xc8,0x68,0x6c,0x33,0x144}
def op_tree(v,depth=0,seen=None,budget=None):
    if budget is None:budget=[80]
    if budget[0]<=0:return {"kind":"budget-limit"}
    budget[0]-=1
    if v is None:return {"kind":"null"}
    if seen is None:seen=set()
    item={"node":str(v)[:110]}
    if v.isConstant():
        item.update({"kind":"constant","value":H(v.getOffset())})
        return item
    if v.isAddress():
        item["storage"]=str(v.getAddress())
    op=v.getDef()
    if op is None:
        item["kind"]="unknown-input"
        return item
    identity=(str(v),str(op.getSeqnum().getTarget()))
    if identity in seen:
        item["kind"]="loop"
        return item
    item["kind"]=str(op.getMnemonic())
    item["site"]=str(op.getSeqnum().getTarget())
    item["inputs"]=[
        op_tree(op.getInput(i),depth+1,seen|{identity},budget)
        for i in range(min(op.getNumInputs(),6))]
    return item
def op_consts(v,depth=0,seen=None):
    """Conservative literal collection, not a physical-address resolver."""
    if v is None or depth>10:return set()
    if seen is None:seen=set()
    key=str(v)
    if key in seen:return set()
    seen=seen|{key}
    if v.isConstant():return {int(v.getOffset())&0xffffffff}
    op=v.getDef()
    if op is None:return set()
    vals=set()
    for i in range(min(op.getNumInputs(),7)):
        vals|=op_consts(op.getInput(i),depth+1,seen)
    return vals
def instruction_row(address):
    inst=listing.getInstructionContaining(A(address))
    if inst is None:return None
    return {"pc":H(address),"assembly":str(inst)[:220],
            "flow_targets":[str(x) for x in inst.getFlows()][:9]}
def xref(value,max_entries=200):
    results=[]
    for ref in refs.getReferencesTo(A(value)):
        src=fm.getFunctionContaining(ref.getFromAddress())
        results.append({
          "pc":H(ref.getFromAddress().getOffset()),
          "function":H(src.getEntryPoint().getOffset()) if src else None,
          "type":str(ref.getReferenceType())
        })
        if len(results)>=max_entries:break
    return results
# Full current Ghidra direct call graph with exact source addresses.
edges=defaultdict(set)
reverse=defaultdict(set)
callsites=defaultdict(list)
for entry,fn in funcs.items():
    it=listing.getInstructions(fn.getBody(),True)
    while it.hasNext():
        ins=it.next()
        if ins.getFlowType().isCall():
            for target in ins.getFlows():
                dest=fm.getFunctionContaining(target)
                if dest is None:continue
                dst=int(dest.getEntryPoint().getOffset())
                if dst not in funcs:continue
                edges[entry].add(dst);reverse[dst].add(entry)
                callsites[(entry,dst)].append(H(ins.getAddress().getOffset()))
def reach(root,graph):
    q=deque([(root,0)])
    result={root:0}
    while q:
        node,depth=q.popleft()
        for dest in graph.get(node,()):
            if dest not in result:
                result[dest]=depth+1
                q.append((dest,depth+1))
    return result
root_entries={}
for name,at in KEY.items():
    fn=fm.getFunctionContaining(A(at))
    if fn:
        root_entries[name]=int(fn.getEntryPoint().getOffset())
priority={}
for name,entry in root_entries.items():
    for side in (reach(entry,edges),reach(entry,reverse)):
        for target,depth in side.items():
            priority[target]=min(priority.get(target,1000000),depth)
started=time.monotonic()
stats=Counter()
selected=[]
decompiled=[]
errors=[]
storage_methods=[]
indirect=[]
rerefs=[]
outfile=folder/"V20-P3-BACKEND-COMMAND-FUNCTIONS.jsonl"
dec=DecompInterface()
dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(P):raise RuntimeError("Ghidra decompiler cannot open owner program")
try:
    with outfile.open("w") as sink:
        for index,entry in enumerate(sorted(funcs,key=lambda x:(priority.get(x,1000000),x))):
            fn=funcs[entry]
            result_row={
                "entry":H(entry),"name":str(fn.getName()),
                "root_distance":priority.get(entry),
                "calls":[{"to":H(k),"sites":callsites[(entry,k)]}
                         for k in sorted(edges.get(entry,()))],
                "callback_fields":[],"indirect_calls":[],
                "interesting_loads":[],"errors":[]
            }
            try:
                result=dec.decompileFunction(fn,25,monitor)
                if result is None or not result.decompileCompleted() or result.getHighFunction() is None:
                    raise RuntimeError(str(result.getErrorMessage()) if result else "No result")
                pseudocode=str(result.getDecompiledFunction().getC())
                pseudocode=re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+",
                                   "inmusic-unlock-magic-[REDACTED]",pseudocode)
                if entry in root_entries.values():
                    result_row["pseudocode"]=pseudocode[:26000]
                elif priority.get(entry,1000000)<=2:
                    result_row["pseudocode"]=pseudocode[:2800]
                ops=result.getHighFunction().getPcodeOps()
                while ops.hasNext():
                    op=ops.next()
                    kind=str(op.getMnemonic())
                    if kind not in ("STORE","LOAD","CALLIND","BRANCHIND","PTRADD","PTRSUB"):
                        continue
                    pc=int(op.getSeqnum().getTarget().getOffset())
                    if kind in ("STORE","LOAD"):
                        loc_idx=1 if op.getNumInputs()>1 else 0
                        target=op.getInput(loc_idx)
                    else:
                        target=op.getInput(0) if op.getNumInputs()>0 else None
                    consts=op_consts(target)
                    match=consts & ROOT_OFFSETS
                    row={"pc":H(pc),"kind":kind,"literal_operands":[H(x) for x in sorted(consts)[:25]],
                         "relevant_offsets":[H(x) for x in sorted(match)],
                         "instruction":instruction_row(pc)}
                    if kind in ("CALLIND","BRANCHIND"):
                        row["target_dependency"]=op_tree(target,max_depth=11) if False else op_tree(target)
                        result_row["indirect_calls"].append(row)
                        indirect.append({"function":H(entry),"site":H(pc),"dependency":row["target_dependency"]})
                        stats["indirect_sites"]+=1
                    elif kind=="STORE" and (match or entry in root_entries.values()):
                        row["location_dependency"]=op_tree(target)
                        value=op.getInput(2) if op.getNumInputs()>2 else None
                        row["stored_value_dependency"]=op_tree(value)
                        result_row["callback_fields"].append(row)
                        storage_methods.append({"function":H(entry),"site":H(pc),
                                                "offsets":row["relevant_offsets"],"kind":"STORE"})
                        stats["store_relevant"]+=1
                    elif kind=="LOAD" and (match or entry in root_entries.values()):
                        row["address_dependency"]=op_tree(target)
                        result_row["interesting_loads"].append(row)
                        stats["load_relevant"]+=1
                stats["decompiled"]+=1
                decompiled.append(H(entry))
            except Exception as exc:
                error=str(exc)[:450]
                result_row["errors"].append(error)
                errors.append({"entry":H(entry),"error":error})
                stats["decompile_failed"]+=1
            sink.write(json.dumps(result_row)+"\n")
            if (index+1)%100==0:
                sink.flush()
                print("V20 P3 callback/boot census",index+1,"/",len(funcs),
                      "decompiled",stats["decompiled"],
                      "indirect",stats["indirect_sites"],
                      "field_STORES",stats["store_relevant"],
                      "time",round(time.monotonic()-started,1))
finally:
    dec.dispose()
# Prove separate interprocedural facts with exact cited instruction witnesses.
backend=funcs.get(root_entries.get("persist_entry"))
provider=funcs.get(root_entries.get("backend_object"))
specific=[]
if backend:
    it=listing.getInstructions(backend.getBody(),True)
    while it.hasNext():
        ins=it.next()
        at=int(ins.getAddress().getOffset())
        if 0x0010b790<=at<=0x0010b7e4:
            specific.append(instruction_row(at))
struct={
 "backend_persistent_entry":"0x0010b74c",
 "backend_factory":"0x0011c3ac",
 "backend_object_projection":"0x0011c3ec",
 "backend_object_projection_expected_delta":200,
 "vtable_member_offset_from_object":"0x00000068",
 "derived_backend_callback_offset":"0x00000130",
 "testable_claim":"Object b is projected to b+0xc8; original code reads callback at b+0x130.",
 "static_proof_sites":specific,
 "callsite_in_real_vendor_p3":"0x0010b7d6",
 "DO_NOT_INFER":"Callback may reach eMMC/SD; its actual provider is unresolved until verified."
}
summary={
 "schema":"mpfree-v20-full-p3-field-and-command-census",
 "owner_sha256":expected,
 "owner_input_verified":True,
 "timestamp_utc":datetime.now(timezone.utc).isoformat(),
 "functions_recognized":len(funcs),
 "functions_attempted":len(funcs),
 "functions_decompiled":stats["decompiled"],
 "failures":errors,
 "call_edges":sum(len(v) for v in edges.values()),
 "indirect_sites":stats["indirect_sites"],
 "store_relevant":stats["store_relevant"],
 "load_relevant":stats["load_relevant"],
 "callback_field_stores":storage_methods[:1000],
 "indirect_call_inventory":indirect[:1000],
 "critical_seed_functions":{k:H(v) for k,v in root_entries.items()},
 "recursive_call_closures":{
    name:{
      "callees":len(reach(root,edges)),"callers":len(reach(root,reverse)),
      "direct_callees":[H(t) for t in sorted(edges.get(root,()))],
      "direct_callers":[H(t) for t in sorted(reverse.get(root,()))]
    } for name,root in root_entries.items()
 },
 "storage_vtable_offset_model":struct,
 "source_function_report":outfile.name,
 "elapsed_seconds":round(time.monotonic()-started,2),
 "limitations":[
   "All computed storage vtable offsets are static evidence, not physical eMMC writes.",
   "One recognized function can contain multiple primitive CALLIND sites.",
   "Function pointer registrations may be constructed dynamically or relocated.",
   "No physical target, updater, Fastboot transport, or owner storage is executed."
 ]
}
(folder/"V20-P3-BACKEND-COMMAND-SUMMARY.json").write_text(json.dumps(summary,indent=2)+"\n")
print("V20 P3 finished",stats["decompiled"],"/",len(funcs),
      "callback field stores",stats["store_relevant"],
      "indirect calls",stats["indirect_sites"])
