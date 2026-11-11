# @runtime PyGhidra
# @category MPFree
# Audit all P1/P3 functions not decompiled by the preceding V3 graph pass.
# Exhaustive coverage within the recognized Ghidra function graph, NOT
# claim that Ghidra found every real function or every indirect branch.
# ONLY existing hash-verified projects: -readOnly -noanalysis.
import hashlib
import json
import os
import re
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte
SOURCES={
  "p1":(0xff704000,32768,"6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
  "p3":(0x00100000,363649,"b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"),
}
args=list(getScriptArgs())
if len(args)!=3 or args[0] not in SOURCES:
    raise RuntimeError("Expected p1|p3 /absolute/v3.json /absolute/v4.json")
kind=str(args[0]); prior=Path(str(args[1])); outpath=Path(str(args[2]))
if not prior.is_absolute() or not outpath.is_absolute():
    raise RuntimeError("Both report paths must be absolute")
old=json.loads(prior.read_text(encoding="utf-8"))
base,size,expected=SOURCES[kind]
if old.get("schema")!="mpfree-evidence-graph-v3" or old.get("kind")!=kind or old.get("sha256")!=expected:
    raise RuntimeError("Refusing mismatched prior full-graph report")
p=currentProgram
if p is None or str(p.getLanguageID())!="ARM:LE:32:v7":raise RuntimeError("Wrong CPU architecture")
mem=p.getMemory(); space=p.getAddressFactory().getDefaultAddressSpace()
A=lambda n:space.getAddress(int(n))
H=lambda n:"0x%08x"%int(n)
block=mem.getBlock(A(base))
if block is None or block.getStart().getOffset()!=base or block.getSize()!=size:
    raise RuntimeError("Wrong Ghidra payload size")
buf=JArray(JByte)(32768);raw=bytearray()
for off in range(0,size,32768):
    n=min(32768,size-off)
    if int(mem.getBytes(A(base+off),buf,0,n))!=n:
        raise RuntimeError("Truncated payload")
    raw.extend(int(x)&255 for x in buf[:n])
if hashlib.sha256(raw).hexdigest()!=expected:raise RuntimeError("Ghidra binary hash mismatch")
fm=p.getFunctionManager(); listing=p.getListing();refs=p.getReferenceManager()
graph=old["all_known_function_graph"]
already=set(old["detailed_functions"])
missing=[name for name in graph if name not in already]
callers=defaultdict(set)
for source,row in graph.items():
    for target in row.get("calls",[]):
        callers[target].add(source)
# Resolve each V3 research-group seed into its Ghidra containing function.
function_tags=defaultdict(set)
for group,meta in old["groups"].items():
    for address in meta["anchors"]+meta["extra_callers_callees"]:
        try:
            fn=fm.getFunctionContaining(A(int(address,16)))
            if fn:
                function_tags[H(fn.getEntryPoint().getOffset())].add(group)
        except Exception:
            pass
# Tag neighbors of the important anchors; do not label all adjacent functions
# as proven reachable on actual hardware.
for source,tags in list(function_tags.items()):
    for target in graph.get(source,{}).get("calls",[]):
        function_tags[target].update(tags)
    for caller in callers.get(source,[]):
        function_tags[caller].update(tags)
def priority(address):
    v=graph[address]
    tags=function_tags.get(address,set())
    return (35*int(bool(tags))+
            min(20,len(callers.get(address,set()))*2)+
            min(20,len(v.get("calls",[])))+
            18*len(v.get("computed_calls",[]))+
            (18 if any("fastboot" in t or "bootrom" in t or
                       "reset_mode" in t or "security" in t for t in tags) else 0))
queue=sorted(missing,key=lambda addr:(-priority(addr),addr))
dec=DecompInterface()
dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(p):raise RuntimeError("Decompiler failed")
report={
 "schema":"mpfree-recognized-function-coverage-v4",
 "kind":kind,"date_utc":datetime.now(timezone.utc).isoformat(),
 "mode":"offline; owner-verified P1/P3 existing read-only Ghidra project",
 "source_payload_sha256":expected,
 "recognized_functions":len(graph),"previously_detailed":len(already),
 "previously_undetailed":len(missing),
 "coverage":{},"remaining":[],
 "limitations":[
    "Recognized-function coverage is not comprehensive binary instruction coverage.",
    "Computed branch targets and ARM/Thumb modes can be misidentified by Ghidra.",
    "No code execution, ROM readout, hardware probe, eFuse writes or flash.",
    "Reports contain proprietary decompilation: keep private, do not commit.",
 ],
}
try:
    for address in queue:
        fun=fm.getFunctionAt(A(int(address,16)))
        if fun is None:
            report["remaining"].append({"address":address,"reason":"Ghidra function missing"})
            continue
        row=graph[address]
        details={"address":address,"name":str(fun.getName()),"priority":priority(address),
                 "research_tags":sorted(function_tags.get(address,set())),
                 "computed_call_sites":row.get("computed_calls",[]),
                 "calls":row.get("calls",[]),"caller_count":len(callers.get(address,set()))}
        # Capture short exact instruction windows around every computed
        # callsite to expose vtable/driver callback structure.
        if row.get("computed_calls"):
            details["computed_call_disassembly"]=[]
            for call in row["computed_calls"][:16]:
                pc=int(call,16)
                start=max(base,pc-20)
                ins=listing.getInstructionContaining(A(start))
                if ins is None:ins=listing.getInstructionAfter(A(start))
                lines=[]
                while ins and ins.getAddress().getOffset()<pc+30 and len(lines)<22:
                    lines.append({"at":str(ins.getAddress()),"asm":str(ins)})
                    ins=ins.getNext()
                details["computed_call_disassembly"].append({"site":call,"instructions":lines})
        try:
            d=dec.decompileFunction(fun,60,monitor)
            if d and d.decompileCompleted() and d.getDecompiledFunction():
                src=str(d.getDecompiledFunction().getC())
                src=re.sub(r'"[^"\n]{42,}"','"[LONG STRING REDACTED]"',src)
                src=re.sub(r'"[^"\n]*(?:password|unlock|token|secret|credential|serialno|ethaddr)[^"\n]*"',
                           '"[SENSITIVE STRING REDACTED]"',src,flags=re.I)
                details["pseudocode"]=src[:15000]
                details["pseudocode_truncated"]=len(src)>15000
            else:
                details["decompile_error"]=str(d.getErrorMessage()) if d else "No decompiler result"
        except Exception as error:
            details["decompile_error"]=str(error)
        report["coverage"][address]=details
finally:dec.dispose()
report["newly_detailed"]=len(report["coverage"])
report["cumulative_detailed"]=len(already)+len(report["coverage"])
report["uncovered_function_entries"]=len(report["remaining"])
outpath.parent.mkdir(parents=True,exist_ok=True)
umask=os.umask(0o077)
try:outpath.write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")
finally:os.umask(umask)
print("MPFree exhaustive recognized-function V4:",kind,
      "previous",len(already),"new",len(report["coverage"]),
      "total",len(graph),"failures",len(report["remaining"]),outpath)
