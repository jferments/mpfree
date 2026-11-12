# @runtime PyGhidra
# @category MPFree
"""V17 fixed-point, all-recognized-function Ghidra call/dataflow investigation.

Read-only saved project, pinned SRAM/U-Boot SHA. Every recognized function is
examined, every high-level P-code STORE/LOAD/CALLIND/CBRANCH is indexed,
every direct call closure and reverse-caller closure is traced to a fixed
point, not an arbitrary two/four-hop depth. No physical hardware access.
"""
import hashlib,json,os,time,re
from collections import defaultdict,deque,Counter
from pathlib import Path
from datetime import datetime,timezone
from ghidra.app.decompiler import DecompInterface
from jpype import JArray,JByte

PINNED={
 "p1":(0xff704000,32768,"6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
 "p3":(0x00100000,363649,"b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a")
}
ROOTS={
 "p1":{
  "maskrom_spl_handoff":[0xff704004,0xff704060,0xff7040b4,0xff705208],
  "media_probe_and_mmc":[0xff705310,0xff705350,0xff705398,0xff7053c2,0xff7054a4],
  "recovery_pmu":[0xff705316,0xff7056a8,0xff708b74,0xff708cdc,0xff708f98],
  "ddr_loader":[0xff707f48,0xff7052c0,0xff705598,0xff7055a4],
 },
 "p3":{
  "oem_auth_and_console":[0x00122834,0x00122854,0x00110484,0x001095f8],
  "fastboot_receive_and_download":[0x00122980,0x00122a10,0x0012247c,0x00121dd0],
  "flash_erase_storage_backend":[0x00122c5c,0x00122bf8,0x00122bd0,0x001100c8,0x00110270],
  "ram_boot_bootm_bootz":[0x001226b4,0x00122964,0x00101744,0x0010a6a4,0x0010e390],
  "image_signature_validation":[0x0010e000,0x0010e390,0x0010f388,0x00127a1c],
  "environment_persistence":[0x00108560,0x0010b74c,0x0010d588,0x0010ccb8],
  "sd_mmc_boot_targets":[0x00100fe4,0x001011e0,0x001095f8,0x00110484],
  "pmu_button_recovery":[0x00100fe4,0x001147f0,0x0011622a,0x0011b5d0],
 }
}
TOKENS=("boot_targets","bootcmd","bootz","bootm","fastboot","oem","mmc","sd",
 "emmc","fdt","image","crc","signature","usb","env","saveenv","reset","bootrom",
 "recovery","flash","erase","write","read","verify","update","loader","memory",
 "partition","reloc","ramdisk","rockusb")
argv=list(getScriptArgs())
if len(argv) not in (2,3) or str(argv[0]) not in PINNED:
    raise RuntimeError("Usage: p1|p3 ABSOLUTE_PRIVATE_DIRECTORY [TEMP_PRIVATE_EXPORT]")
role=str(argv[0]);out=Path(str(argv[1]))
if not out.is_absolute():raise RuntimeError("Requires absolute private output directory")
base,size,sha=PINNED[role]
P=currentProgram
if P is None or str(P.getLanguageID())!="ARM:LE:32:v7":
    raise RuntimeError("Wrong source architecture")
space=P.getAddressFactory().getDefaultAddressSpace()
def A(i):return space.getAddress(int(i))
def H(i):return "0x%08x"%int(i)
memory=P.getMemory();b=memory.getBlock(A(base))
if b is None or int(b.getStart().getOffset())!=base or int(b.getSize())!=size:
    raise RuntimeError("Wrong saved Ghidra address range/size")
buf=JArray(JByte)(65536);raw=bytearray()
for start in range(0,size,65536):
    take=min(65536,size-start)
    if int(memory.getBytes(A(base+start),buf,0,take))!=take:
        raise RuntimeError("Saved Ghidra image truncated")
    raw.extend(int(i)&255 for i in buf[:take])
raw=bytes(raw)
if hashlib.sha256(raw).hexdigest()!=sha:
    raise RuntimeError("Saved Ghidra image did not match verified owner P1/P3")
# A private verified executable page is made available to QEMU IMMEDIATELY,
# so the VM boots and probes while the full Ghidra dataflow pass continues.
if len(argv)==3:
    export=Path(str(argv[2]))
    if role!="p3" or not export.is_absolute() or export.exists() or export.is_symlink():
        raise RuntimeError("Private code export permitted only for P3 at a new absolute temp path")
    export.parent.mkdir(parents=True,exist_ok=True)
    old_umask=os.umask(0o077)
    try:
        with export.open("xb") as f:
            f.write(raw);f.flush();os.fsync(f.fileno())
        os.chmod(str(export),0o600)
        ready=export.with_name(export.name+".ready")
        with ready.open("x") as f:
            f.write(sha+"\n");f.flush();os.fsync(f.fileno())
    finally:os.umask(old_umask)

listing=P.getListing();fm=P.getFunctionManager();refs=P.getReferenceManager()
functions=list(fm.getFunctions(True))
lookup={int(f.getEntryPoint().getOffset()):f for f in functions}
edges=defaultdict(set);reverse=defaultdict(set)
unresolved=[]
def addref(src,dst,where,source):
    target=fm.getFunctionContaining(A(dst))
    if target is None:return
    entry=int(target.getEntryPoint().getOffset())
    if entry not in lookup:return
    edges[src].add(entry);reverse[entry].add(src)
# Build complete direct-call graph and an explicit unresolved indirect-call queue.
for addr,fn in lookup.items():
    ins=listing.getInstructions(fn.getBody(),True)
    while ins.hasNext():
        inst=ins.next()
        if not inst.getFlowType().isCall():continue
        targets=[int(x.getOffset()) for x in inst.getFlows()]
        if not targets:
            unresolved.append({"caller":H(addr),"site":H(inst.getAddress().getOffset()),
                               "asm":str(inst)[:180],"resolution":"UNKNOWN_INDIRECT"})
        for target in targets:addref(addr,target,inst.getAddress(),"instruction")
def bfs(starts,graph):
    dist={int(n):0 for n in starts};parent={int(n):None for n in starts}
    todo=deque(int(n) for n in starts)
    while todo:
        node=todo.popleft()
        for nxt in graph.get(node,()):
            if nxt not in dist:
                dist[nxt]=dist[node]+1
                parent[nxt]=node
                todo.append(nxt)
    return dist,parent
domains={}
priority={}
for name,addresses in ROOTS[role].items():
    starts=set()
    for addr in addresses:
        fn=fm.getFunctionContaining(A(addr))
        if fn:starts.add(int(fn.getEntryPoint().getOffset()))
    fwd,fp=bfs(starts,edges);back,bp=bfs(starts,reverse)
    domains[name]={
        "root_functions":[H(a) for a in sorted(starts)],
        "forward_fixed_point_reachable":[{"entry":H(a),"distance":d,
             "parent":H(fp[a]) if fp[a] is not None else None}
             for a,d in sorted(fwd.items(),key=lambda x:(x[1],x[0]))],
        "reverse_fixed_point_callers":[{"entry":H(a),"distance":d,
             "parent":H(bp[a]) if bp[a] is not None else None}
             for a,d in sorted(back.items(),key=lambda x:(x[1],x[0]))],
        "newly_unreachable_known_functions":len(lookup)-len(fwd)
    }
    for m in (fwd,back):
        for k,d in m.items():priority[k]=min(priority.get(k,1000000),d)
    for x in starts:priority[x]=0
# Function-accurate reciprocal xrefs and Ghidra P-code effects.
target_ids=sorted(lookup,key=lambda a:(priority.get(a,1000000),a))
dec=DecompInterface();dec.toggleCCode(True);dec.setSimplificationStyle("decompile")
if not dec.openProgram(P):raise RuntimeError("Unable to open SHA-verified Ghidra project")
os.umask(0o077);out.mkdir(parents=True,exist_ok=True)
details=out/("V17-"+role+"-HIGH-PCODE-FUNCTIONS.jsonl")
effects=out/("V17-"+role+"-INDIRECT-AND-MMIO.jsonl")
count=Counter();interesting_nodes=[]
start_time=time.monotonic()
def varnode(v):
    if v is None:return None
    x={"text":str(v)[:120],"size":int(v.getSize())}
    if v.isConstant():
        x["const"]=H(v.getOffset()) if v.getOffset()<0x100000000 else str(v.getOffset())
    if v.isAddress():x["address"]=str(v.getAddress())
    try:
        d=v.getDef()
        if d is not None:
            x["def_opcode"]=d.getMnemonic()
            x["def_site"]=str(d.getSeqnum().getTarget())
            if d.getNumInputs()<=4:
                x["def_inputs"]=[str(d.getInput(j))[:100] for j in range(d.getNumInputs())]
    except Exception:pass
    return x
try:
    with details.open("w") as doc,effects.open("w") as effect_out:
        for idx,entry in enumerate(target_ids):
            fn=lookup[entry]
            doc_row={"entry":H(entry),"name":str(fn.getName()),
                     "body_addresses":int(fn.getBody().getNumAddresses()),
                     "domain_distance":priority.get(entry),
                     "direct_callees":[H(x) for x in sorted(edges.get(entry,()))],
                     "direct_callers":[H(x) for x in sorted(reverse.get(entry,()))],
                     "pcode_effects":[],"branch_decisions":[],"failure":None}
            t=time.monotonic()
            try:
                res=dec.decompileFunction(fn,25,monitor)
                if not res or not res.decompileCompleted() or not res.getHighFunction():
                    raise RuntimeError(str(res.getErrorMessage()) if res else "no result")
                code=str(res.getDecompiledFunction().getC())
                code=re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+",
                            "inmusic-unlock-magic-[REDACTED]",code)
                # Full pseudocode for root-neighborhood and indirect dispatchers.
                depth=priority.get(entry,1000000)
                cap=16000 if depth<=2 else (5000 if depth<=5 else 900)
                doc_row["pseudocode"]=code[:cap]
                doc_row["pseudocode_total_bytes"]=len(code)
                doc_row["pseudocode_truncated"]=len(code)>cap
                hf=res.getHighFunction()
                ops=hf.getPcodeOps()
                op_count=0
                # Emit every key high-level operation; no 700/1000-op truncation.
                while ops.hasNext():
                    op=ops.next();op_count+=1
                    key=op.getMnemonic()
                    if key in ("LOAD","STORE","CALL","CALLIND","RETURN","PTRADD",
                               "PTRSUB","INT_ADD","INT_SUB","INT_AND",
                               "INT_EQUAL","INT_NOTEQUAL","INT_LESS","INT_LESSEQUAL",
                               "INT_SLESS","INT_SLESSEQUAL",
                               "CBRANCH","BRANCHIND","MULTIEQUAL","INDIRECT"):
                        inputs=[varnode(op.getInput(j)) for j in range(op.getNumInputs())]
                        row={"pc":str(op.getSeqnum().getTarget()),"kind":key,
                             "in":inputs,"out":varnode(op.getOutput())}
                        if key in ("CALLIND","BRANCHIND") and inputs:
                            candidates=possible_constant_targets(op.getInput(0))
                            hits=[]
                            for value in sorted(candidates):
                                target_fn=fm.getFunctionContaining(A(value&~1))
                                if target_fn and int(target_fn.getEntryPoint().getOffset()) in lookup:
                                    hits.append(H(target_fn.getEntryPoint().getOffset()))
                            row["static_indirect_target_candidates"]=sorted(set(hits))
                            row["candidate_not_runtime_proof"]=True
                            if hits:count["indirect_candidate_target_sites"]+=1
                        if key=="STORE" and op.getNumInputs()>=2:
                            values=possible_constant_targets(op.getInput(1))
                            row["candidate_address_constants"]=[H(v) for v in sorted(values)[:24]]
                            if any(v in (0xff730094,0xff7300a0,0x0014d690) for v in values):
                                count["writes_relevant_pmureg_or_auth_global"]+=1
                        doc_row["pcode_effects"].append(row)
                        count["pcode_"+key]+=1
                        if key in ("STORE","CALLIND","BRANCHIND"):
                            effect_out.write(json.dumps({"function":H(entry),**row})+"\n")
                        if key in ("CBRANCH","INT_EQUAL","INT_NOTEQUAL","INT_LESS",
                                   "INT_LESSEQUAL","INT_SLESS","INT_SLESSEQUAL"):
                            doc_row["branch_decisions"].append(row)
                count["pcode_ops_total"]+=op_count
                count["function_decompiled"]+=1
                matches=[w for w in TOKENS if w in code.lower() or w in str(fn.getName()).lower()]
                if matches:
                    doc_row["topic_matches"]=matches
                    interesting_nodes.append({"entry":H(entry),"topics":matches,
                                               "domain_distance":depth})
            except Exception as exc:
                doc_row["failure"]=str(exc)[:600]
                count["decompile_failed"]+=1
            doc_row["seconds"]=round(time.monotonic()-t,4)
            doc.write(json.dumps(doc_row)+"\n")
            if (idx+1)%100==0:
                doc.flush();effect_out.flush()
                print("V17",role,"decompiled",idx+1,"/",len(target_ids),
                      "P-code",count["pcode_ops_total"],"elapsed",round(time.monotonic()-start_time,1))
finally:
    dec.dispose()
# Exhaustively scan inline constants pointing to code and selected strings.
pointer_candidates=[]
tables=[]
for off in range(0,len(raw)-4,4):
    word=int.from_bytes(raw[off:off+4],"little")
    if (word&~1) in lookup:
        pointer_candidates.append({"at":H(base+off),"target":H(word&~1),"thumb":bool(word&1)})
# Consecutive function pointers: candidate callback tables, no assumptions.
last=None;current=[]
for row in pointer_candidates:
    at=int(row["at"],16)
    if last is not None and at==last+4:current.append(row)
    else:
        if len(current)>=2:tables.append(current)
        current=[row]
    last=at
if len(current)>=2:tables.append(current)
# Ghidra-defined non-auth strings and both data/code references.
strings=[]
it=listing.getDefinedData(True)
while it.hasNext():
    datum=it.next()
    try:
        val=datum.getValue()
        if not isinstance(val,(str,)) and val.__class__.__name__ not in ("String","JString"):
            continue
        val=str(val)
        if len(val)<3 or len(val)>220:continue
        addr=int(datum.getAddress().getOffset())
        if ("unlock-magic" in val or re.search(r"(?i)pass(word)?|token|serial|secret",val)):
            val="<REDACTED_OWNER_OR_AUTH_STRING>"
        xref=[]
        for r in refs.getReferencesTo(datum.getAddress()):
            xref.append(H(r.getFromAddress().getOffset()))
            if len(xref)>100:break
        strings.append({"address":H(addr),"value":val,"refs":xref})
    except Exception:continue
# New evidence: distinguish all discovered inline function pointer tables,
# MMIO/reboot candidates and indirect dispatch sites.
def possible_constant_targets(v,depth=0,seen=None):
    """Bounded recursive Ghidra High P-code backtrace. Candidates, NOT proofs."""
    if v is None or depth>=9:return set()
    if seen is None:seen=set()
    identity=id(v)
    if identity in seen:return set()
    seen=seen|{identity}
    if v.isConstant():return {int(v.getOffset())&0xffffffff}
    defn=v.getDef()
    if defn is None:return set()
    op=str(defn.getMnemonic())
    if op in ("COPY","CAST","INT_ZEXT","INT_SEXT","SUBPIECE","INDIRECT"):
        if defn.getNumInputs():return possible_constant_targets(defn.getInput(0),depth+1,seen)
    if op=="MULTIEQUAL":
        union=set()
        for j in range(min(8,defn.getNumInputs())):
            union|=possible_constant_targets(defn.getInput(j),depth+1,seen)
        return set(list(union)[:24])
    if op in ("INT_ADD","PTRSUB","INT_SUB","PTRADD"):
        if defn.getNumInputs()<2:return set()
        left=possible_constant_targets(defn.getInput(0),depth+1,seen)
        right=possible_constant_targets(defn.getInput(1),depth+1,seen)
        if not left or not right:return set()
        multiplier=1
        if op=="PTRADD" and defn.getNumInputs()>=3:
            vals=possible_constant_targets(defn.getInput(2),depth+1,seen)
            if vals:multiplier=min(vals)
        combined=set()
        for a in list(left)[:12]:
            for b in list(right)[:12]:
                v=(a-b if op=="INT_SUB" else a+b*multiplier)&0xffffffff
                combined.add(v)
        return set(list(combined)[:24])
    if op=="LOAD" and defn.getNumInputs()>=2:
        addresses=possible_constant_targets(defn.getInput(1),depth+1,seen)
        return {int.from_bytes(raw[a-base:a-base+4],"little")
                for a in addresses if base<=a<=base+size-4}
    return set()
report={
 "schema":"mpfree-v17-full-fixedpoint-high-pcode-v1",
 "role":role,"timestamp_utc":datetime.now(timezone.utc).isoformat(),
 "source_sha256":sha,"verified_owner":True,
 "functions_known":len(lookup),"functions_attempted":len(target_ids),
 "direct_unique_edges":sum(len(v) for v in edges.values()),
 "unresolved_instruction_calls":unresolved,
 "statistics":dict(count),
 "elapsed_seconds":round(time.monotonic()-start_time,2),
 "domain_graphs":domains,
 "candidate_function_pointer_tables":tables,
 "candidate_pointer_sites":pointer_candidates,
 "defined_strings":strings,
 "topic_function_index":interesting_nodes,
 "function_report":details.name,
 "dataflow_report":effects.name,
 "limits":[
  "Fixed-point covers Ghidra-recognized code, not unknown unrecognized functions.",
  "High P-code references do not prove physical state or runtime callbacks.",
  "Unknown CALLIND targets must not be treated as absent.",
  "Recursive constant propagation produces candidates only, not real runtime edges.",
  "No vendor updater, physical device or firmware was executed."
 ]
}
(out/("V17-"+role+"-SUMMARY.json")).write_text(json.dumps(report,indent=2)+"\n")
print("V17 completed",role,"functions",len(lookup),
      "decompiled",count["function_decompiled"],
      "pcode",count["pcode_ops_total"],
      "unresolved_indirect",len(unresolved),"tables",len(tables))
