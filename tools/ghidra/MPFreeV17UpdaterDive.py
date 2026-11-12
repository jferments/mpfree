# @runtime PyGhidra
# @category MPFree
"""V17 exhaustive cross-caller / callback / verification graph of verified ARM updater ELFs.

Runs only inside a TEMPORARY Ghidra import project; ELF is NEVER executed.
The launcher must SHA-verify original raw ELF first. Do not mark Ghidra
relocated memory SHA as equivalent to original ELF bytes.
"""
import os,re,json,time
from collections import defaultdict,deque,Counter
from datetime import datetime,timezone
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
argv=list(getScriptArgs())
if len(argv)!=3 or str(argv[0]) not in ("libaz01-prog","az01-update"):
    raise RuntimeError("Usage: libaz01-prog|az01-update ABSOLUTE_PRIVATE_DIR VERIFIED_SOURCE_SHA")
role=str(argv[0]);out=Path(str(argv[1]));sha=str(argv[2])
if not out.is_absolute() or len(sha)!=64 or not re.fullmatch("[0-9a-f]{64}",sha):
    raise RuntimeError("Refusing updater analysis without host-verifiable source SHA")
P=currentProgram
if P is None or not str(P.getLanguageID()).startswith("ARM:LE:32"):
    raise RuntimeError("Verified updater must import as ARM little-endian ELF")
listing=P.getListing();fm=P.getFunctionManager();refs=P.getReferenceManager()
functions=list(fm.getFunctions(True))
lookup={int(f.getEntryPoint().getOffset()):f for f in functions}
def H(v):return "0x%08x"%int(v)
def A(i):return P.getAddressFactory().getDefaultAddressSpace().getAddress(int(i))
edges=defaultdict(set);rev=defaultdict(set);unknown=[]
for k,fn in lookup.items():
    ins=listing.getInstructions(fn.getBody(),True)
    while ins.hasNext():
        op=ins.next()
        if not op.getFlowType().isCall():continue
        targets=op.getFlows()
        if not targets:
            unknown.append({"caller":H(k),"at":str(op.getAddress()),
                            "assembly":str(op),"kind":"indirect/relocated"})
        for x in targets:
            fn2=fm.getFunctionContaining(x)
            if fn2:
                target=int(fn2.getEntryPoint().getOffset())
                if target in lookup:
                    edges[k].add(target);rev[target].add(k)
def bfs(seeds,graph):
    seen=set(seeds);queue=deque(seeds);parent={x:None for x in seeds}
    while queue:
        a=queue.popleft()
        for b in graph.get(a,()):
            if b not in seen:
                seen.add(b);parent[b]=a;queue.append(b)
    return parent
# Recursively explore all functions connected to update/write and signature roots;
# then analyze every remaining function to find previously unseen call paths.
terms={
 "transport_and_protocol":("rockusb","fastboot","usb","bulk","send","recv","device"),
 "signature_and_image":("verify","signature","sha","check","format","header","image","fit"),
 "write_commit":("write","erase","partition","flash","commit","sector","bootloader"),
 "backend_virtual_dispatch":("backend","vtable","callback","protocol","open","close"),
 "restore_recovery":("reboot","bootrom","reset","maskrom","loader","recovery"),
}
groups={}
for key,words in terms.items():
    seeds=[a for a,f in lookup.items() if any(w in str(f.getName()).lower() for w in words)]
    forward=bfs(seeds,edges);backward=bfs(seeds,rev)
    groups[key]={"seeds":[H(x) for x in seeds],
        "full_forward_closure":[{"function":H(x),"parent":H(y) if y is not None else None}
                                for x,y in sorted(forward.items())],
        "full_reverse_caller_closure":[{"function":H(x),"parent":H(y) if y is not None else None}
                                for x,y in sorted(backward.items())]}
out.mkdir(parents=True,exist_ok=True);os.umask(0o077)
doc=out/("V17-"+role+"-ELF-HIGH-PCODE.jsonl")
dec=DecompInterface();dec.toggleCCode(True);dec.setSimplificationStyle("decompile")
if not dec.openProgram(P):raise RuntimeError("Could not open updater ELF for decompilation")
stat=Counter();elapsed=time.monotonic()
try:
    with doc.open("w") as f:
        for at,fn in sorted(lookup.items()):
            row={"entry":H(at),"function":str(fn.getName()),
                 "direct_callees":[H(x) for x in sorted(edges.get(at,()))],
                 "direct_callers":[H(x) for x in sorted(rev.get(at,()))],
                 "pcode":[],"pseudocode":None}
            try:
                res=dec.decompileFunction(fn,25,monitor)
                if not res or not res.decompileCompleted() or not res.getHighFunction():
                    raise RuntimeError(str(res.getErrorMessage()) if res else "empty")
                code=str(res.getDecompiledFunction().getC())
                row["pseudocode"]=code[:10000]
                row["pseudocode_truncated"]=len(code)>10000
                stat["functions_decompiled"]+=1
                ops=res.getHighFunction().getPcodeOps()
                while ops.hasNext():
                    op=ops.next()
                    kind=op.getMnemonic()
                    stat["pcode_ops_total"]+=1
                    if kind not in ("CALL","CALLIND","BRANCHIND","CBRANCH",
                                    "LOAD","STORE","MULTIEQUAL","INDIRECT",
                                    "PTRADD","PTRSUB","INT_ADD","INT_AND",
                                    "INT_EQUAL","INT_NOTEQUAL","INT_LESS","INT_LESSEQUAL"):
                        continue
                    inputs=[]
                    for j in range(op.getNumInputs()):
                        var=op.getInput(j)
                        x={"name":str(var)[:140]}
                        if var.isConstant():x["constant"]=H(var.getOffset())
                        try:
                            d=var.getDef()
                            if d is not None:
                                x["defined_by"]=d.getMnemonic()
                                x["definition_site"]=str(d.getSeqnum().getTarget())
                        except Exception:pass
                        inputs.append(x)
                    row["pcode"].append({"address":str(op.getSeqnum().getTarget()),
                                         "kind":kind,"inputs":inputs})
                    stat["pcode_"+kind]+=1
            except Exception as e:
                row["decompile_error"]=str(e)[:500];stat["failed"]+=1
            f.write(json.dumps(row)+"\n")
finally:dec.dispose()
# Global discovery: unresolved callbacks and class/vtable candidate pointers.
data_words=[];memory=P.getMemory();blocks=list(memory.getBlocks())
for block in blocks:
    if not block.isInitialized() or not block.isRead():continue
    try:
        start=int(block.getStart().getOffset());total=int(block.getSize())
        if total>32*1024*1024:continue
        from jpype import JArray,JByte
        chunk=JArray(JByte)(min(total,65536));image=bytearray()
        for offset in range(0,total,len(chunk)):
            n=min(len(chunk),total-offset)
            read=int(memory.getBytes(A(start+offset),chunk,0,n))
            if read!=n:raise ValueError("Incomplete ELF memory block")
            image.extend(int(x)&255 for x in chunk[:n])
        for ofs in range(0,len(image)-4,4):
            value=int.from_bytes(image[ofs:ofs+4],"little")&~1
            if value in lookup:
                data_words.append({"address":H(start+ofs),"function":H(value)})
    except Exception:continue
tables=[];run=[];last=None
for row in sorted(data_words,key=lambda x:int(x["address"],16)):
    pos=int(row["address"],16)
    if last is not None and pos==last+4:run.append(row)
    else:
        if len(run)>=2:tables.append(run)
        run=[row]
    last=pos
if len(run)>=2:tables.append(run)
summary={
 "schema":"mpfree-v17-updater-elf-recursive-v1",
 "role":role,"source_sha256_preverified_by_owner_host":sha,
 "owner_elf_executed":False,"physical_hardware_contact":False,
 "ghidra_import_memory_not_raw_elf":True,
 "functions_recognized":len(lookup),"function_report":doc.name,
 "call_graph_edges":sum(map(len,edges.values())),
 "unresolved_calls":unknown,
 "recursive_groups":groups,"candidate_callback_tables":tables,
 "candidate_function_pointers":data_words[:8000],
 "statistics":dict(stat),"elapsed_seconds":round(time.monotonic()-elapsed,2),
 "limitations":[
  "ELF imported in a temporary Ghidra project with owner SHA checked by runner.",
  "Indirect targets and backend virtual dispatch remain hypotheses without relocation/dataflow proof.",
  "No real RockUSB, Fastboot, updater, storage or MCU hardware ever executes."
 ]}
(out/("V17-"+role+"-SUMMARY.json")).write_text(json.dumps(summary,indent=2)+"\n")
print("V17 updater deep",role,"recognized",len(lookup),
      "decompiled",stat["functions_decompiled"],
      "pcode",stat["pcode_ops_total"],"indirect",len(unknown),"tables",len(tables))
