# @runtime PyGhidra
# @category MPFree
"""V26 SPL: decode original P1 five-entry boot-media preference table.

Prior scripts recorded a literal pointer 0xff7053fc -> 0xff709c70
and a 5-iteration media-selection loop at 0xff705350, but did NOT
decode the five actual table words, their mode classification, and
the four source handler descriptors. This script reads pinned 32KiB
original P1 bytes, compares the source instructions and emits the
full recognized-function direct-call graph + detailed media functions.
Read-only saved Ghidra project, no physical MPC or firmware write.
"""
from ghidra.app.decompiler import DecompInterface
from jpype import JArray,JByte
from pathlib import Path
from collections import defaultdict,deque,Counter
from datetime import datetime,timezone
import hashlib,json,os

args=list(getScriptArgs())
if len(args)!=1 or not Path(str(args[0])).is_absolute():
    raise RuntimeError("Expected /abs/private-output")
out=Path(str(args[0]));out.mkdir(parents=True,exist_ok=True)
os.umask(0o077)
P=currentProgram
BASE=0xff704000;SIZE=32768
SHA="6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"
if P is None or str(P.getLanguageID())!="ARM:LE:32:v7":
    raise RuntimeError("Owner P1 project wrong language")
space=P.getAddressFactory().getDefaultAddressSpace()
def A(n):return space.getAddress(int(n)&0xffffffff)
def H(n):return "0x%08x"%(int(n)&0xffffffff)
mem=P.getMemory();blk=mem.getBlock(A(BASE))
if blk is None or blk.getStart().getOffset()!=BASE or blk.getSize()!=SIZE:
    raise RuntimeError("Owner first-stage SRAM block unexpected")
buf=JArray(JByte)(32768)
if int(mem.getBytes(A(BASE),buf,0,SIZE))!=SIZE:
    raise RuntimeError("Cannot recover full P1 owner bytes")
raw=bytes(int(x)&255 for x in buf)
if hashlib.sha256(raw).hexdigest()!=SHA:
    raise RuntimeError("First-stage SHA pin mismatch")
def word(at):
    pos=at-BASE
    return int.from_bytes(raw[pos:pos+4],"little") if 0<=pos<=SIZE-4 else None
def read_ascii(at,maxlen=140):
    pos=at-BASE
    if pos<0 or pos>=SIZE:return None
    end=raw.find(b"\0",pos,min(SIZE,pos+maxlen))
    if end<pos:return None
    val=raw[pos:end]
    return val.decode("ascii",errors="replace") if all(32<=x<=126 for x in val) else None
table=word(0xff7053fc)
kind=word(0xff705404)
terminator=word(0xff7053f8)
if not (table and kind and BASE<=table<BASE+SIZE-20 and
        BASE<=kind<BASE+SIZE-40 and terminator==0xdeadbeef):
    raise RuntimeError("SPL source table/sentinel different from prior verified Ghidra")
order=[word(table+4*i) for i in range(5)]
types=[word(kind+4*i) for i in range(10)]
if any(x is None for x in order+types):
    raise RuntimeError("Owner priority list/descriptor truncated")
# The original five slots are all 0xDEADBEEF in saved ROM.
# FUN_ff705342 writes the return value of FUN_ff704120 into the
# FIRST slot at runtime. Do NOT mislabel a sentinel array as boot priority.
preinit_slots=list(order)
def classify(id_):
    if id_==types[0]:return 0
    if id_==types[2]:return 1
    if id_==types[4]:return 2
    return 3
entries=[]
for idx,v in enumerate(order):
    cls=classify(v)
    choice=types[cls*2+1]
    entries.append({
       "position":idx,"literal_media_selector_value":H(v),
       "type_group":cls,"group_source_value":H(types[cls*2]),
       "owner_handler_argument":H(choice),
       "handler_argument_statically_resolved":True,
       "physical_SD_or_eMMC_controller_UNPROVEN":True})
fm=P.getFunctionManager();listing=P.getListing()
fns={int(f.getEntryPoint().getOffset()):f for f in fm.getFunctions(True)}
refman=P.getReferenceManager()
roots=[
  0xff705350,0xff705316,0xff708f98,0xff7054a4,0xff705310,
  0xff705298,0xff705208,0xff7052c0,0xff708b74,0xff708cdc,
  0xff7056a8,0xff705342,0xff704120,0xff7041a8,
  0xff705418,0xff70604a,0xff708e10,0xff705b16,0xff705af2,0xff7089fc,0xff708770
]
edges=defaultdict(set);back=defaultdict(set);sites=defaultdict(list)
for address,fn in fns.items():
    it=listing.getInstructions(fn.getBody(),True)
    while it.hasNext():
        ins=it.next()
        if not ins.getFlowType().isCall():continue
        for dest in ins.getFlows():
            f=fm.getFunctionContaining(dest)
            if f:
                callee=int(f.getEntryPoint().getOffset())
                if callee in fns:
                    edges[address].add(callee);back[callee].add(address)
                    sites[(address,callee)].append(H(ins.getAddress().getOffset()))
def reach(entry,graph):
    q=deque([entry]);seen={entry:0}
    while q:
        node=q.popleft()
        for next_ in graph.get(node,()):
            if next_ not in seen:
                seen[next_]=seen[node]+1;q.append(next_)
    return seen
focus={}
for root in roots:
    fn=fm.getFunctionContaining(A(root))
    if not fn:continue
    entry=int(fn.getEntryPoint().getOffset())
    for graph in (edges,back):
        for k,dep in reach(entry,graph).items():
            focus[k]=min(dep,focus.get(k,100000))
stats=Counter();failed=[]
dec=DecompInterface()
dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(P):raise RuntimeError("P1 source decoder failed")
outpath=out/"V26-P1-ALL-FUNCTIONS-AND-BOOT-MEDIA.jsonl"
try:
    with outpath.open("w") as f:
        for entry,fn in sorted(fns.items()):
            row={"entry":H(entry),"name":str(fn.getName()),
                 "nearest_media_root_hops":focus.get(entry),
                 "direct_callees":[H(x) for x in sorted(edges.get(entry,()))],
                 "callsites":[{"to":H(x),"at":sites[(entry,x)]}
                              for x in sorted(edges.get(entry,()))],
                 "pcode_effects":[]}
            try:
                r=dec.decompileFunction(fn,30,monitor)
                if not r or not r.decompileCompleted() or not r.getHighFunction():
                    raise RuntimeError(str(r.getErrorMessage()) if r else "no decompile result")
                stats["decompiled"]+=1
                if entry in roots or focus.get(entry,999)>0 and focus.get(entry,999)<=2:
                    row["pseudocode"]=str(r.getDecompiledFunction().getC())[:12000]
                ops=r.getHighFunction().getPcodeOps()
                while ops.hasNext():
                    op=ops.next();k=str(op.getMnemonic())
                    if k not in ("STORE","CALLIND","BRANCHIND","LOAD"):continue
                    stats[k]+=1
                    if entry in roots or (focus.get(entry,999)<=2 and k!="LOAD"):
                        row["pcode_effects"].append({
                          "kind":k,"site":str(op.getSeqnum().getTarget()),
                          "inputs":[str(op.getInput(i))[:110]
                                    for i in range(min(op.getNumInputs(),6))]})
            except Exception as exc:
                row["error"]=str(exc)[:350]
                failed.append({"entry":H(entry),"error":row["error"]})
            f.write(json.dumps(row)+"\n")
finally:dec.dispose()
xref=[]
for site in (0xff7053fc,table,0xff705404,kind):
    refs=[]
    for ref in refman.getReferencesTo(A(site)):
        refs.append({"site":H(ref.getFromAddress().getOffset()),
                     "kind":str(ref.getReferenceType())})
    xref.append({"location":H(site),"read_from_image":H(word(site)),"xrefs":refs[:180]})
result={
 "schema":"mpfree-v26-original-P1-SPL-five-entry-media-order",
 "owner_SHA256":SHA,"owner_verified":True,
 "generated_utc":datetime.now(timezone.utc).isoformat(),
 "source_media_selection_function":"0xff705350",
 "five_entry_priority_ptr_cell":"0xff7053fc",
 "five_entry_array":H(table),
 "terminator":H(terminator),
 "kind_table_ptr_cell":"0xff705404",
 "kind_table":H(kind),
 "priority_entries":entries,
 "raw_priority_entries_are_all_uninitialized_sentinel":
     all(v==0xdeadbeef for v in order),
 "actual_boot_priority_statically_known":False,
 "dynamic_first_entry_initialized_by":"FUN_ff705342",
 "dynamic_first_entry_source":"FUN_ff704120",
 "dynamic_return_source_needs_High_Pcode_and_board_boot_state":True,
 "full_kind_table":[{"offset":i*4,"word":H(v)} for i,v in enumerate(types)],
 "fifth_and_other_modes_not_mapped_to_physical_controllers":True,
 "sentinel_cross_check":len(entries)==5 and terminator==0xdeadbeef,
 "pointer_source_xrefs":xref,
 "recognized_functions":len(fns),
 "decompiled_functions":stats["decompiled"],
 "functions_with_Ghidra_errors":failed,
 "direct_call_edges":sum(len(v) for v in edges.values()),
 "op_statistics":dict(stats),
 "source":"V26-P1-ALL-FUNCTIONS-AND-BOOT-MEDIA.jsonl",
 "limitation":"SPL priority values and handler IDs do not determine actual SD or eMMC physical identity without code mapping and board tests."
}
# New source-origin deep audit: traverse all original instructions and
# decompiled High P-code operations in the selector, its caller and
# initialization function. The original code reads board/ROM state at
# runtime, so exact priority cannot be inferred solely from 5 literals.
critical=(0xff704120,0xff705342,0xff705350,0xff705418,
          0xff705316,0xff708b74,0xff708cdc,0xff7041a8,
          0xff705b16,0xff705af2,0xff7089fc,0xff708770)
source_rows=[]
deep=DecompInterface();deep.toggleCCode(True)
deep.setSimplificationStyle("decompile")
if not deep.openProgram(P):raise RuntimeError("P1 deep selector source unavailable")
try:
    for entry in critical:
        fn=fm.getFunctionContaining(A(entry))
        if not fn:continue
        actual=int(fn.getEntryPoint().getOffset())
        asm=[]
        it=listing.getInstructions(fn.getBody(),True)
        while it.hasNext():
            ins=it.next()
            if len(asm)>=2500:break
            asm.append({"pc":H(ins.getAddress().getOffset()),
                        "assembly":str(ins),
                        "flow":[str(t) for t in ins.getFlows()]})
        row={"requested":H(entry),"function_entry":H(actual),
             "assembly":asm,"decompiled":False}
        try:
            res=deep.decompileFunction(fn,40,monitor)
            if res is None or not res.decompileCompleted() or not res.getHighFunction():
                raise RuntimeError(str(res.getErrorMessage()) if res else "empty")
            row["decompiled"]=True
            row["pseudocode"]=str(res.getDecompiledFunction().getC())[:90000]
            row["pcode"]=[]
            ops=res.getHighFunction().getPcodeOps()
            while ops.hasNext():
                op=ops.next()
                row["pcode"].append({
                   "kind":str(op.getMnemonic()),
                   "site":str(op.getSeqnum().getTarget()),
                   "inputs":[str(op.getInput(j))[:300] for j in
                             range(min(op.getNumInputs(),8))],
                   "output":str(op.getOutput())[:300] if op.getOutput() else None})
        except Exception as exc:
            row["error"]=str(exc)[:700]
        source_rows.append(row)
finally:deep.dispose()
source_audit={
    "schema":"mpfree-v26-SPL-runtime-media-source-origin",
    "owner_P1_SHA256":SHA,
    "source_initialized_at_runtime":True,
    "preinit_array":[H(v) for v in preinit_slots],
    "all_entries_sentinel":all(v==0xdeadbeef for v in preinit_slots),
    "dynamic_write_target":"0xff709c70",
    "dynamic_write_function":"0xff705342",
    "dynamic_value_function":"0xff704120",
    "source_value_static_range_from_owner_decompilation":[1,8],
    "special_condition_comparison_value":"0x35",
    "special_condition_origin_function":"0xff705b16",
    "source_value_1_means_physical_eMMC":False,
    "source_value_8_means_physical_SD":False,
    "loop":"0xff705350",
    "full_decompiled_source_and_instruction_context":source_rows,
    "hardware_sampling_not_executed_in_Ghidra":True,
    "exact_physical_SD_eMMC_order_proven":False
}
(out/"V26-P1-DYNAMIC-BOOT-SOURCE-ORIGIN.json").write_text(
    json.dumps(source_audit,indent=2)+"\n")

(out/"V26-P1-SPL-BOOT-MEDIA-ORDER.json").write_text(json.dumps(result,indent=2)+"\n")
print("V26 SPL source order resolved",len(entries),"entries",
      "and decompiled",stats["decompiled"],"/",len(fns),"functions",flush=True)
