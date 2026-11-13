# @runtime PyGhidra
# @category MPFree
"""MPFree V24: original block-write/erase callback and low-level driver transport.

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
   "native_Uboot_command_registry":[0x00110ccc,0x00110d2c,0x001111e0,
                                   0x00110d48,0x001095f8,0x00110484],
   "native_MMC_device_index_and_info":[0x00104f88,0x00105620,
                                       0x0011c3ac,0x0011c3f0,0x0011daf4],
   "genuine_MMC_read_and_partition":[0x00105958,0x00105a48,0x00105acc,
                                     0x00111c7c,0x00104fd4],
   "filesystem_loading_and_bootz":[0x00106000,0x00105fcc,0x00105f00,
                                   0x00101744,0x0010a6a4,0x001018fc],
   "autoboot_environment_and_provenance":[0x00108804,0x00108560,
                                         0x00108434,0x001085b4,
                                         0x001006e4,0x00100768],
   "dfu_mmc_is_USB_updater_not_direct_boot":[0x00116f64,0x00116ec8,
                                            0x00116c8c,0x0011689c],
   "actual_controller_registration":[0x0011c964,0x0011c9a8,
                                     0x0011d8e8,0x0011c434,
                                     0x0011c4e8,0x0011c912],
 },
  "p1":{
  "MaskROM_and_first_stage_entry":[0xff704004,0xff704060,0xff704068,
                                  0xff7040b4,0xff7040f4],
  "first_stage_MMC_media_selector":[0xff705310,0xff705350,0xff705398,
                                    0xff7053c2,0xff7054a4,0xff708f98],
  "SPL_owner_loader_handoff":[0xff705208,0xff7052c0,0xff705308,
                               0xff7055b0,0xff7055bc,0xff705444],
  "bootloader_USB_recovery_branches":[0xff705316,0xff7056a8,
                                      0xff708b74,0xff708cdc,0xff707f48],
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
  "registered_read_fnptr_literal":0x0011d958,
  "registered_write_fnptr_literal":0x0011d95c,
  "registered_erase_fnptr_literal":0x0011d960,
  "driver_transfer_function_table_word0":0x0011c9a0,
  "driver_transfer_function_table_word1":0x0011c9a4,
  "driver_vtable_entry0":0x0012f8f4,
  "driver_vtable_entry1":0x0012f8f8,
  "driver_table_global_literal":0x0011c9a0,
  "backend_registry_head_literal":0x0011c3d4,
  "native_command_table_start_pointer":0x00110d3c,
  "native_command_table_end_pointer":0x00110d40,
  "native_command_table_count_multiplier":0x00110d44,
  "MMC_device_registry_counter":0x0011c3f8,
  "current_MMC_selection":0x00105828,
 },
 "p1":{
   "SPL_media_priority_end_marker":0xff7053f8,
   "SPL_media_priority_table":0xff7053fc,
   "SPL_media_status_message":0xff705400,
   "SPL_media_kind_table":0xff705404,
   "SPL_media_handler_descriptor":0xff705408,
   "SPL_fallback_handler_pointer":0xff70540c,
   "SPL_retry_message_pointer":0xff705410,
   "SPL_termination_status_pointer":0xff705414,
   "SPL_read_buffer_pointer":0xff705598,
   "SPL_read_failure_message":0xff70559c,
   "SPL_stage_handoff_pointer":0xff705274,
   "SPL_first_stage_memory_end":0xff705270,
 }
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
    print("V22 P3 private ARM image ready; QEMU may start concurrently")

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
# Extract REAL cmd_tbl_t-like U-Boot records directly from the verified
# owner firmware instead of treating a string occurrence as a command.
# FUN_00110CCC accesses seven 32-bit words per descriptor, and the
# FUN_00110D2C wrapper carries original owner start/end pointers.
# Sensitive/unrecognized names are SHA-indexed, never printed directly.
def v24_u32(at):
    if base<=at<=base+size-4:
        return int.from_bytes(raw[at-base:at-base+4],"little")
    return None
def v24_ascii(at,n=64):
    if not isinstance(at,int) or not(base<=at<base+size):
        return None
    i=at-base
    j=raw.find(b"\0",i,min(i+n+1,len(raw)))
    if j<=i or j-i>n:return None
    data=raw[i:j]
    return data if all(32<=c<=126 for c in data) else None
COMMAND_ALLOWLIST=set(("mmc","mmcinfo","load","ls","ext4load","fatload",
 "ext4ls","fatls","bootz","bootm","booti","boot","source","sysboot",
 "run","env","printenv","setenv","saveenv","help","echo","test",
 "fdt","part","usb","dfu","fastboot","reset","reboot","version",
 "crc32","iminfo","imxtract","setexpr","sleep","md","mw","cp","erase",
 "sf","nand","blk","fstype","pci","gpio","dhcp","tftpboot","ping"))
registry={"schema":"mpfree-v24-real-owner-uboot-command-descriptors",
          "all_source_names_redacted_except_allowlist":True,
          "entries":[],"layout_verified":False,"candidate_handler_roots":[]}
if role=="p3":
    first=v24_u32(0x00110d3c)
    last=v24_u32(0x00110d40)
    multiplier=v24_u32(0x00110d44)
    registry["first"]=H(first) if first is not None else None
    registry["last"]=H(last) if last is not None else None
    registry["multiplier"]=H(multiplier) if multiplier is not None else None
    if (first is not None and last is not None and
        base<=first<last<=base+size and
        (last-first)%28==0 and 2<=(last-first)//28<=512):
        n=(last-first)//28
        registry["descriptor_count"]=n
        handlers=[]
        for i in range(n):
            where=first+i*28
            fields=[v24_u32(where+4*j) for j in range(7)]
            name=v24_ascii(fields[0])
            real_name=name.decode("ascii") if name is not None else None
            safe=real_name if real_name in COMMAND_ALLOWLIST else None
            destination=fields[3]
            target=fm.getFunctionContaining(A(destination&~1)) if destination else None
            valid=target is not None
            entry={
              "descriptor_address":H(where),
              "allowlisted_command_name":safe,
              "redacted_name_SHA256":hashlib.sha256(name).hexdigest() if name else None,
              "name_length":len(name) if name else None,
              "max_arguments":fields[1],
              "repeatable_flag":fields[2],
              "handler_pointer":H(destination) if destination else None,
              "recognized_handler":H(target.getEntryPoint().getOffset()) if target else None,
              "help_pointer_readable":bool(v24_ascii(fields[5]))
            }
            registry["entries"].append(entry)
            if safe and valid:handlers.append(destination&~1)
        registry["layout_verified"]=bool(
            n>=5 and sum(bool(r["recognized_handler"]) for r in registry["entries"])>=n//2)
        if registry["layout_verified"]:
            GROUPS["p3"]["registered_SD_and_boot_command_handlers"]=handlers
            registry["candidate_handler_roots"]=[H(x) for x in sorted(set(handlers))]
    else:
        registry["layout_issue"]="Pointer range not an in-image 28-byte descriptor table; no assumed U-Boot capabilities"

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
    table=image_u32(0x0011c9a0)
    first=image_u32(0x0012f8f4)
    second=image_u32(0x0012f8f8)
    if table!=0x0012f8f4 or first!=0x0011c4e9 or second!=0x0011c913:
        raise RuntimeError("Physical owner driver function table not what V21 raw-pointer inventory established")
    literal_evidence["mmc_driver_table_mapping"]={
        "table_pointer_cell":"0x0011c9a0",
        "table_address":H(table),
        "command_member_cell":"0x0012f8f4",
        "command_thumb_pointer":H(first),
        "command_handler":"0x0011c4e8",
        "config_member_cell":"0x0012f8f8",
        "config_thumb_pointer":H(second),
        "config_handler":"0x0011c912",
        "dispatcher":"0x0011ca78",
        "command_register_fields":[H(x) for x in [0x18,0x1c,0x20,0x28,0x2c,0x44,0x48,0x74,0x80,0x88]],
        "driver_backend_layout":"backend+8 -> driver_context, driver_context+4 -> vtable, vtable[0] -> command handler",
        "real_MMC_host_selected":False,
        "all_evidence_static_except_separate_QEMU_tests":True
    }
    write_pointer=image_u32(0x0011d95c)
    erase_pointer=image_u32(0x0011d960)
    if write_pointer is None or (write_pointer&~1)!=0x0011de50:
        raise RuntimeError("Owner P3 registered-write callback pointer mismatch")
    if erase_pointer is None or (erase_pointer&~1)!=0x0011dd4c:
        raise RuntimeError("Owner P3 registered-erase callback pointer mismatch")
    literal_evidence["registered_backend_callbacks_verified"]={
      "registration_function":"0x0011d8e8",
      "backend_object_size":0x148,
      "backend_list_registration":"0x0011c434",
      "backend_interface_projection":"0x0011c3ec",
      "interface_delta":"0x000000c8",
      "write_callback_interface_offset":"0x00000068",
      "write_callback_effective_object_offset":"0x00000130",
      "write_literal":"0x0011d95c",
      "write_pointer":H(write_pointer),
      "actual_block_write_function":"0x0011de50",
      "erase_literal":"0x0011d960",
      "erase_pointer":H(erase_pointer),
      "erase_function":"0x0011dd4c",
      "transport_indirect_function":"0x0011ca78",
      "static_evidence_only":True,
      "real_physical_backend_target_unknown":True
    }
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
filename=report_dir/("V24-"+role+"-PATH-WITNESSES.jsonl")
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
                print("V22 detailed path witness",role,index+1,"of",len(focus),
                      "STORE",stats["deep_STORE"],"CALLIND",stats["deep_CALLIND"])
finally:
    dec.dispose()
summary={
    "schema":"mpfree-v24-native-command-registry-and-media-v1",
    "original_command_registry":registry,
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
(report_dir/("V24-"+role+"-SUMMARY.json")).write_text(json.dumps(summary,indent=2)+"\n")
if role=="p3":
    (report_dir/"V24-COMMAND-REGISTRY.json").write_text(json.dumps(registry,indent=2)+"\n")
print("MPFree V24",role,"traced",len(focus),
      "functions, typed STORE",stats["deep_STORE"],
      "typed CALLIND",stats["deep_CALLIND"],
      "failures",len(failed))
