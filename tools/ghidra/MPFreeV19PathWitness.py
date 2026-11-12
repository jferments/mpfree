# @runtime PyGhidra
# @category MPFree
"""MPFree V19: provenance-preserving interprocedural path witnesses.

This is a READ-ONLY analysis of hash-pinned owner's saved ARM Ghidra project.
Unlike V18's shallow constant propagation, every relevant LOAD/STORE/CALLIND
records a recursively expanded expression tree identifying actual varnode
storage locations, dereferences, arithmetic, phi nodes and unresolved inputs.
Conclusions must distinguish a code literal, a pointer value and a physical
MMIO address. Only temporary P3 bytes are exported for isolated QEMU.
"""
import hashlib
import json
import os
from collections import Counter,defaultdict,deque
from datetime import datetime,timezone
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
from jpype import JArray,JByte

IMAGES={
 "p1":(0xff704000,32768,
       "6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
 "p3":(0x00100000,363649,
       "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a")
}
GROUPS={
 "p3":{
  "getvar_env_and_partition_fallback":[0x00122d38,0x00108804,0x00100ef0,
                                       0x0012dc3a,0x0012dcb2,0x0012dd3e],
  "reboot_mode_adc_recovery":[0x00100fe4,0x00100edc,0x001147f0,
                              0x00115ef4,0x00115f30,0x001147d4,
                              0x0011622a],
  "fastboot_auth_oem_shell":[0x00122834,0x00122854,0x00110484,0x001095f8],
  "env_write_storage_backend":[0x0010b74c,0x0011c3ac,0x0011c3ec,
                               0x0011daf4,0x0010cdf4,0x00108560],
  "download_ram_image_handoff":[0x00122980,0x00122a10,0x001226b4,
                                0x0010d14c,0x00101744,0x0010e390],
  "mmc_sd_boot_script":[0x001011e0,0x00106000,0x001095f8,
                        0x00110484,0x0010e000,0x0010e390],
 },
 "p1":{
  "first_stage_boot_media":[0xff704060,0xff705350,0xff705310,
                            0xff705316,0xff7054a4,0xff708f98],
  "handoff_rom":[0xff705208,0xff7055b0,0xff7055bc,0xff705444],
  "recovery_and_sram":[0xff7056a8,0xff708b74,0xff708cdc,0xff707f48],
 }
}
WATCH={
 "p3":{
  "auth_literal_reader":0x0012284c,
  "auth_literal_writer":0x00122928,
  "reboot_mode_literal":0x0010114c,
  "recovery_flag_literal":0x00101154,
  "adc_status_literal":0x0010113c,
  "getvar_environment_name_literal":0x00122e60,
  "getvar_partition_lookup_literal":0x00122e70,
  "download_size_literal":0x001229f4,
  "download_count_literal":0x00122ac4,
  "download_total_literal":0x00122ad4,
  "fastboot_boot_image_literal":0x001226fc,
 },
 "p1":{}
}
argv=list(getScriptArgs())
if len(argv)!=4:raise RuntimeError("usage: p1|p3 /abs/report-dir /abs/raw-or-none /abs/ready-or-none")
role=str(argv[0]);report_dir=Path(str(argv[1]))
raw_file=str(argv[2]);ready_file=str(argv[3])
if role not in IMAGES or not report_dir.is_absolute():
    raise RuntimeError("Invalid role or report directory")
if role=="p1" and (raw_file!="none" or ready_file!="none"):
    raise RuntimeError("P1 may not export firmware to guest")
if role=="p3" and (not Path(raw_file).is_absolute() or
                   not Path(ready_file).is_absolute()):
    raise RuntimeError("P3 guest staging paths must be absolute")
base,size,expected=IMAGES[role]
P=currentProgram
if P is None or str(P.getLanguageID())!="ARM:LE:32:v7":
    raise RuntimeError("Expected saved ARM:LE:32:v7 program")
space=P.getAddressFactory().getDefaultAddressSpace()
def A(value):return space.getAddress(int(value)&0xffffffff)
def H(value):return "0x%08x"%(int(value)&0xffffffff)
mem=P.getMemory();block=mem.getBlock(A(base))
if block is None or block.getStart().getOffset()!=base or block.getSize()!=size:
    raise RuntimeError("Unexpected owner image address/length")
buf=JArray(JByte)(32768);raw=bytearray()
for offset in range(0,size,32768):
    count=min(size-offset,32768)
    if int(mem.getBytes(A(base+offset),buf,0,count))!=count:
        raise RuntimeError("Cannot read complete firmware input")
    raw.extend(int(x)&255 for x in buf[:count])
raw=bytes(raw)
if hashlib.sha256(raw).hexdigest()!=expected:
    raise RuntimeError("Saved owner image source SHA-256 mismatch")
report_dir.mkdir(parents=True,exist_ok=True)
os.umask(0o077)
if role=="p3":
    raw_path=Path(raw_file);marker=Path(ready_file)
    if raw_path.is_symlink() or marker.is_symlink():
        raise RuntimeError("Refuse symlink for private guest export")
    raw_path.parent.mkdir(parents=True,exist_ok=True)
    with raw_path.open("xb") as f:
        f.write(raw);f.flush();os.fsync(f.fileno())
    if hashlib.sha256(raw_path.read_bytes()).hexdigest()!=expected:
        raise RuntimeError("Private Ghidra guest source export failed integrity")
    marker.write_text(expected+"\n")
    print("V19 P3 private ARM image ready; QEMU may start concurrently")

fm=P.getFunctionManager();listing=P.getListing();refs=P.getReferenceManager()
lookup={int(fn.getEntryPoint().getOffset()):fn for fn in fm.getFunctions(True)}
outgoing=defaultdict(set);incoming=defaultdict(set)
for entry,fn in lookup.items():
    instructions=listing.getInstructions(fn.getBody(),True)
    while instructions.hasNext():
        ins=instructions.next()
        if not ins.getFlowType().isCall():continue
        for target in ins.getFlows():
            callee=fm.getFunctionContaining(target)
            if callee:
                addr=int(callee.getEntryPoint().getOffset())
                if addr in lookup:
                    outgoing[entry].add(addr);incoming[addr].add(entry)
def fixed_reach(starts,graph):
    pending=deque(starts);depth={x:0 for x in starts}
    while pending:
        a=pending.popleft()
        for b in graph.get(a,()):
            if b not in depth:
                depth[b]=depth[a]+1
                pending.append(b)
    return depth
root_groups={};priority={}
for domain,targets in GROUPS[role].items():
    roots=set()
    for addr in targets:
        fn=fm.getFunctionContaining(A(addr))
        if fn:
            root=int(fn.getEntryPoint().getOffset())
            if root in lookup:roots.add(root)
    forward=fixed_reach(roots,outgoing)
    backward=fixed_reach(roots,incoming)
    root_groups[domain]={
        "roots":[H(a) for a in sorted(roots)],
        "recursive_callees_count":len(forward),
        "recursive_callers_count":len(backward),
        "recursive_callees":[{"address":H(k),"depth":v}
                             for k,v in sorted(forward.items())],
        "recursive_callers":[{"address":H(k),"depth":v}
                             for k,v in sorted(backward.items())]}
    for source in (forward,backward):
        for k,v in source.items():
            priority[k]=min(priority.get(k,999999),v)
# Reuse V18's verified full program graph; for a new fine-grained trace,
# select every function on a root-to-leaf path within four direct-call hops,
# plus callers up to two hops. Record all more distant reachable nodes
# in root_groups, but don't pretend to have traced their P-code here.
focus={k for k,v in priority.items() if v<=4}
if not focus:raise RuntimeError("No bootchain functions selected for deep tracing")
def image_u32(address):
    if base<=address<=base+size-4:
        return int.from_bytes(raw[address-base:address-base+4],"little")
    return None
def varnode_info(v):
    if v is None:return {"null":True}
    d={"repr":str(v)[:140],"size":int(v.getSize())}
    if v.isConstant():d["constant"]=H(v.getOffset())
    if v.isAddress():
        value=int(v.getAddress().getOffset())
        d["storage_location"]=H(value)
        resident=image_u32(value)
        if resident is not None:d["source_image_word_at_location"]=H(resident)
    return d
def trace(varnode,depth=0,seen=None,max_depth=11,budget=None):
    if budget is None:budget=[160]
    if budget[0]<=0:return {"kind":"node-budget-reached"}
    budget[0]-=1
    if varnode is None:return {"kind":"no-value"}
    if seen is None:seen=set()
    node=varnode_info(varnode)
    identity=(str(varnode),str(varnode.getDef()) if varnode.getDef() else None)
    if depth>=max_depth:
        return {"kind":"truncated-depth","node":node}
    if identity in seen:
        return {"kind":"cyclic-dependency","node":node}
    if varnode.isConstant():
        return {"kind":"constant","node":node}
    op=varnode.getDef()
    if op is None:
        return {"kind":"live-in-or-unknown","node":node}
    kind=str(op.getMnemonic())
    result={
        "kind":kind,"node":node,"def_pc":str(op.getSeqnum().getTarget()),
        "inputs":[]
    }
    for i in range(min(op.getNumInputs(),8)):
        result["inputs"].append(trace(op.getInput(i),depth+1,seen|{identity},max_depth,budget))
    if op.getNumInputs()>8:result["inputs_truncated"]=op.getNumInputs()-8
    return result
def xrefs(at,maximum=100):
    vals=[]
    try:
        for ref in refs.getReferencesTo(A(at)):
            loc=int(ref.getFromAddress().getOffset())
            src=fm.getFunctionContaining(A(loc))
            vals.append({"site":H(loc),
                         "caller_function":H(src.getEntryPoint().getOffset()) if src else None,
                         "reference_type":str(ref.getReferenceType())})
            if len(vals)>=maximum:break
    except Exception as e:vals.append({"exception":str(e)[:300]})
    return vals
def disassembly_site(at,maximum=9):
    ins=listing.getInstructionContaining(A(at))
    if ins is None:ins=listing.getInstructionAfter(A(at))
    if ins is None:return []
    rows=[]
    for _ in range(maximum):
        if ins is None:break
        pos=int(ins.getAddress().getOffset())
        if pos<base or pos>=base+size:break
        rows.append({"site":H(pos),"asm":str(ins)[:220],
                     "flow_targets":[str(x) for x in ins.getFlows()][:8]})
        ins=ins.getNext()
    return rows
literal_evidence={}
for label,address in WATCH[role].items():
    word=image_u32(address)
    literal_evidence[label]={
        "literal_cell":H(address),"static_source_word":H(word) if word is not None else None,
        "incoming_ghidra_xrefs":xrefs(address),
        "target_xrefs":xrefs(word) if word is not None else [],
        "caution":"Read from static file; not proof of the live RAM value."
    }
if role=="p3":
    a=image_u32(WATCH[role]["auth_literal_reader"])
    b=image_u32(WATCH[role]["auth_literal_writer"])
    literal_evidence["auth_pointer_alias"]={
        "both_literals_resolve_same_nonzero_word":bool(a and a==b),
        "gate_read_pointer":H(a) if a is not None else None,
        "oem_write_pointer":H(b) if b is not None else None,
        "scope":"verified static owner P3 only"
    }
dec=DecompInterface();dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(P):raise RuntimeError("Unable to start Ghidra decompiler")
stats=Counter();failed=[]
filename=report_dir/("V19-"+role+"-PATH-WITNESSES.jsonl")
try:
    with filename.open("w") as stream:
        for index,entry in enumerate(sorted(focus,key=lambda k:(priority[k],k))):
            fn=lookup[entry]
            item={
                "entry":H(entry),"name":str(fn.getName()),
                "root_distance":priority[entry],
                "direct_callees":[H(a) for a in sorted(outgoing.get(entry,()))],
                "direct_callers":[H(a) for a in sorted(incoming.get(entry,()))],
                "effects":[],"failed":None
            }
            try:
                result=dec.decompileFunction(fn,30,monitor)
                if not result or not result.decompileCompleted() or not result.getHighFunction():
                    raise RuntimeError(str(result.getErrorMessage()) if result else "No decompiler result")
                code=str(result.getDecompiledFunction().getC())
                import re
                code=re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+",
                            "inmusic-unlock-magic-[REDACTED]",code)
                if priority[entry]<=1:
                    item["pseudocode"]=code[:22000]
                elif priority[entry]<=3:
                    item["pseudocode"]=code[:5000]
                ops=result.getHighFunction().getPcodeOps()
                while ops.hasNext():
                    op=ops.next()
                    kind=str(op.getMnemonic())
                    if kind not in ("STORE","LOAD","CALLIND","BRANCHIND",
                                    "CALL","CBRANCH","INT_EQUAL","INT_NOTEQUAL",
                                    "INT_LESS","INT_SLESS"):
                        continue
                    at=int(op.getSeqnum().getTarget().getOffset())
                    if kind in ("STORE","CALLIND","BRANCHIND"):
                        input_index=1 if kind=="STORE" else 0
                        operand=op.getInput(input_index) if op.getNumInputs()>input_index else None
                        record={
                            "kind":kind,"pc":H(at),
                            "target_expression":trace(operand),
                            "source_instruction":disassembly_site(at,5),
                            "operand_note":"Unknowns retained. No pointer was silently promoted to a physical address."
                        }
                        if kind=="STORE":
                            value=op.getInput(2) if op.getNumInputs()>2 else None
                            record["stored_value_expression"]=trace(value,max_depth=8)
                        item["effects"].append(record)
                        stats["deep_"+kind]+=1
                    elif priority[entry]<=1 and kind in ("CALL","LOAD","CBRANCH"):
                        if op.getNumInputs():
                            item["effects"].append({
                                "kind":kind,"pc":H(at),
                                "source":trace(op.getInput(0),max_depth=7)
                            })
                            stats["root_auxiliary_ops"]+=1
                stats["functions_decompiled"]+=1
            except Exception as e:
                item["failed"]=str(e)[:550];failed.append({"entry":H(entry),"error":item["failed"]})
            stream.write(json.dumps(item)+"\n")
            if (index+1)%25==0:
                stream.flush()
                print("V19 detailed path witness",role,index+1,"of",len(focus),
                      "STORE",stats["deep_STORE"],"CALLIND",stats["deep_CALLIND"])
finally:
    dec.dispose()
summary={
    "schema":"mpfree-v19-directed-interprocedural-pcode-v1",
    "role":role,"owner_sha256":expected,"source_hash_verified":True,
    "generated_utc":datetime.now(timezone.utc).isoformat(),
    "known_functions":len(lookup),"call_edges":sum(len(v) for v in outgoing.values()),
    "root_groups":root_groups,"traced_function_count":len(focus),
    "stats":dict(stats),"decompilation_failures":failed,
    "literal_evidence":literal_evidence,
    "witnesses_file":filename.name,
    "limitations":[
        "Function recognition and High P-code are not actual execution.",
        "Recursive dependency trees can retain unknown runtime parameters.",
        "Source-image pointer literals are not proof of current RAM or hardware state.",
        "Indirect call targets remain unresolved unless supported by instruction/dataflow.",
        "No owner physical MPC, USB, eMMC, OTP or kernel was modified or contacted."
    ]
}
(report_dir/("V19-"+role+"-SUMMARY.json")).write_text(json.dumps(summary,indent=2)+"\n")
print("MPFree V19",role,"traced",len(focus),
      "functions, typed STORE",stats["deep_STORE"],
      "typed CALLIND",stats["deep_CALLIND"],
      "failures",len(failed))
