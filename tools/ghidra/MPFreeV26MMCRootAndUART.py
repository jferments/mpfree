# @runtime PyGhidra
# @category MPFree
"""V26: forensic recovery of EVERY original U-Boot command handler entry.

V24 recovered 86 exact 28-byte native command descriptors, but the saved
Ghidra project recognized only 25 handler pointers and incorrectly marked
the table invalid. This script performs a NEW read-only, code-mode-aware
pseudo-disassembly of ALL 86 handler targets, especially the 61 undefined
Thumb entries. It never modifies/saves the original Ghidra project.

Also decompiles true recognized caller/callee closures of the native MMC
handler, real MMC select/init, filesystem load, image boot and environment
write functions, with typed High P-code address/value dependencies. Full
P1 SPL analysis runs concurrently in the owner host runner.
"""
from ghidra.app.decompiler import DecompInterface
from ghidra.app.util import PseudoDisassembler, PseudoDisassemblerContext
from java.math import BigInteger
from jpype import JArray,JByte
from collections import Counter,defaultdict,deque
from datetime import datetime,timezone
from pathlib import Path
import hashlib,json,os,re,time

argv=list(getScriptArgs())
if len(argv)!=3 or not all(Path(str(x)).is_absolute() for x in argv):
    raise RuntimeError("Usage: /abs/private-reports /abs/private-raw-P3 /abs/ready-marker")
out,rawfile,ready=map(lambda x:Path(str(x)),argv)
out.mkdir(parents=True,exist_ok=True)
os.umask(0o077)
BASE=0x00100000;SIZE=363649
PIN="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
P=currentProgram
if P is None or str(P.getLanguageID())!="ARM:LE:32:v7":
    raise RuntimeError("Owner P3 saved project must be ARM:LE:32:v7")
space=P.getAddressFactory().getDefaultAddressSpace()
def A(n):return space.getAddress(int(n)&0xffffffff)
def H(n):return "0x%08x"%(int(n)&0xffffffff)
m=P.getMemory()
b=m.getBlock(A(BASE))
if b is None or b.getStart().getOffset()!=BASE or b.getSize()!=SIZE:
    raise RuntimeError("Ghidra source is not the verified full P3 block")
buff=JArray(JByte)(32768)
data=bytearray()
for off in range(0,SIZE,32768):
    n=min(32768,SIZE-off)
    if int(m.getBytes(A(BASE+off),buff,0,n))!=n:
        raise RuntimeError("Cannot read owner P3 source in Ghidra")
    data.extend(int(x)&255 for x in buff[:n])
raw=bytes(data)
if hashlib.sha256(raw).hexdigest()!=PIN:
    raise RuntimeError("Wrong owner ROM P3 image; refusing")
if rawfile.is_symlink() or ready.is_symlink():
    raise RuntimeError("Private firmware export refuses symlinks")
rawfile.parent.mkdir(parents=True,exist_ok=True)
with rawfile.open("xb") as f:
    f.write(raw);f.flush();os.fsync(f.fileno())
if hashlib.sha256(rawfile.read_bytes()).hexdigest()!=PIN:
    raise RuntimeError("Guest export hash mismatch")
ready.write_text(PIN+"\n")
print("V26 SHA-verified owner P3 exported for isolated QEMU",flush=True)
def read32(addr):
    pos=addr-BASE
    if pos<0 or pos+4>len(raw):return None
    return int.from_bytes(raw[pos:pos+4],"little")
def safe_ascii(addr):
    pos=addr-BASE
    if pos<0 or pos>=len(raw):return None
    end=raw.find(b"\0",pos,min(pos+96,len(raw)))
    if end<=pos or end-pos>63:return None
    b=raw[pos:end]
    return b if all(32<=x<=126 for x in b) else None
ALLOW=set(("boot","bootm","bootz","mmc","mmcinfo","load","ext4load",
           "fatload","source","sysboot","run","setenv","saveenv",
           "printenv","part","ls","ext4ls","fatls","fdt","help","echo",
           "env","test","dfu","fastboot","reset"))
begin=read32(0x00110d3c);end=read32(0x00110d40)
if not begin or not end or end-begin!=86*28 or begin!=0x0014464c:
    raise RuntimeError("V24 owner command descriptors not 86x28 at source-pinned address")
fm=P.getFunctionManager();listing=P.getListing()
functions={int(f.getEntryPoint().getOffset()):f for f in fm.getFunctions(True)}
pseudo=PseudoDisassembler(P)
pseudo.setMaxInstructions(1800)
pseudo.setRespectExecuteFlag(False)
tmode=P.getProgramContext().getRegister("TMode")
if tmode is None:raise RuntimeError("Owner Ghidra ARM language lacks Thumb TMode")
count=Counter()
report=[]
outfile=out/"V26-COMMAND-HANDLER-THUMB-INSTRUCTIONS.jsonl"
with outfile.open("w") as sink:
    for index in range(86):
        at=begin+index*28
        source=[read32(at+j*4) for j in range(7)]
        name_bytes=safe_ascii(source[0])
        name=name_bytes.decode("ascii") if name_bytes else None
        safe=name if name in ALLOW else None
        pointer=source[3]
        if pointer is None or not (BASE<=pointer&~1<BASE+SIZE):
            raise RuntimeError("Owner registered handler points outside pinned image: "+H(at))
        addr=pointer&~1
        saved=fm.getFunctionContaining(A(addr))
        exact=(saved is not None and
               int(saved.getEntryPoint().getOffset())==addr)
        trace={
          "descriptor":H(at),"allowlisted_name":safe,
          "name_hash":hashlib.sha256(name_bytes).hexdigest() if name_bytes else None,
          "handler_pointer":H(pointer),
          "handler_entry":H(addr),
          "thumb_mode_from_pointer_bit":bool(pointer&1),
          "saved_Ghidra_function_exact":exact,
          "saved_Ghidra_function_containing":H(saved.getEntryPoint().getOffset()) if saved else None,
          "pseudo_instructions":[],"direct_flow_targets":[],
          "instruction_count":0,"errors":[],
          "original_source_SHA":PIN
        }
        if exact:count["already_recognized_exact"]+=1
        else:count["previously_unrecognized"]+=1
        # Never modify or start a transaction on the saved project.
        # Explicitly set Thumb context using PseudoDisassemblerContext.
        q=deque([addr]);visited=set();flow_targets=set()
        while q and len(visited)<400:
            loc=q.popleft()
            if loc in visited or not(BASE<=loc<BASE+SIZE-4):continue
            visited.add(loc)
            try:
                ctx=PseudoDisassemblerContext(P.getProgramContext())
                ctx.flowStart(A(loc))
                ctx.setValue(tmode,BigInteger.ONE if pointer&1 else BigInteger.ZERO)
                instr=pseudo.disassemble(A(loc),ctx,False)
                text=str(instr)
                length=int(instr.getLength())
                flows=[int(x.getOffset()) for x in instr.getFlows()]
                fall=instr.getFallThrough()
                nextaddr=int(fall.getOffset()) if fall is not None else None
                record={"pc":H(loc),"asm":text[:200],
                        "length":length,
                        "flow_type":str(instr.getFlowType()),
                        "explicit_flows":[H(x) for x in flows],
                        "fallthrough":H(nextaddr) if nextaddr is not None else None}
                if len(trace["pseudo_instructions"])<300:
                    trace["pseudo_instructions"].append(record)
                for target in flows:
                    flow_targets.add(target)
                    if BASE<=target<BASE+SIZE-4 and instr.getFlowType().isJump():
                        if target not in visited:q.append(target)
                if nextaddr is not None and BASE<=nextaddr<BASE+SIZE-4:
                    if nextaddr not in visited:q.append(nextaddr)
            except Exception as exc:
                if len(trace["errors"])<8:trace["errors"].append({
                    "pc":H(loc),"error":str(exc)[:240]})
        trace["direct_flow_targets"]=[H(x) for x in sorted(flow_targets)[:300]]
        trace["instruction_count"]=len(trace["pseudo_instructions"])
        trace["truncated_by_400_node_budget"]=len(visited)>=400
        sink.write(json.dumps(trace)+"\n")
        count["pseudo_instructions"]+=trace["instruction_count"]
        if trace["instruction_count"]>0:count["handlers_with_pseudo"]+=1
        if trace["errors"]:count["handlers_with_errors"]+=1
        if (index+1)%10==0:
            sink.flush()
            print("V26 original command handlers",index+1,"/86",
                  "previously unrecognized",count["previously_unrecognized"],
                  "decoded instructions",count["pseudo_instructions"],
                  "handlers with errors",count["handlers_with_errors"],flush=True)
        report.append({
            "descriptor":H(at),"safe_command_name":safe,
            "handler_pointer":H(pointer),"recognized_in_saved_project":exact,
            "pseudo_instruction_count":trace["instruction_count"],
            "first_error":trace["errors"][0] if trace["errors"] else None
        })
# The original native 'mmc' command is itself a command multiplexer:
#   at 0x00104f1c MOVS r2,#10
#   at 0x00104f1e LDR r1,[0x00104f7c] (ten subcommand descriptors)
#   at 0x00104f26 BL FUN_00110CCC (verified owner descriptor lookup).
# Recover exactly these ten *nested* native descriptors and the identity
# of every low-level handler, without dispatching any MMC command.
MMC_SUBCOMMAND_SOURCE_LITERAL=0x00104f7c
sub_begin=read32(MMC_SUBCOMMAND_SOURCE_LITERAL)
SUBCMD_ALLOW=set(("list","dev","info","part","rescan","read","write",
                  "erase","hwpartition","bootbus","bootpart","rpmb",
                  "setdsr","switch","bkops","reliable_write","help"))
subcmd={
  "schema":"mpfree-v26-original-mmc-native-subcommand-table",
  "owner_source_sha256":PIN,
  "literal_cell":H(MMC_SUBCOMMAND_SOURCE_LITERAL),
  "table_address":H(sub_begin) if sub_begin is not None else None,
  "descriptor_count_expected_from_original_machine_code":10,
  "records":[],"all_ten_descriptors_in_owner_rom":False,
  "dangerous_subcommands_may_exist_but_are_NEVER_EXECUTED":True,
  "live_physical_MMC_device_numbering_proven":False
}
if sub_begin is not None and BASE<=sub_begin and sub_begin+10*28<=BASE+SIZE:
    for i in range(10):
        at=sub_begin+28*i
        fields=[read32(at+4*j) for j in range(7)]
        name=safe_ascii(fields[0])
        decoded=name.decode("ascii") if name else None
        permitted=decoded if decoded in SUBCMD_ALLOW else None
        handler=fields[3]
        in_image=(handler is not None and BASE<=(handler&~1)<BASE+SIZE)
        subcmd["records"].append({
           "index":i,"descriptor_at":H(at),
           "safe_name":permitted,
           "name_sha256":hashlib.sha256(name).hexdigest() if name else None,
           "max_argc":fields[1],"repeatable_flag":fields[2],
           "handler_pointer":H(handler) if handler else None,
           "handler_in_original_owner_image":in_image,
           "Thumb_entry":bool(handler&1) if handler else None,
           "Ghidra_saved_function_recognized":bool(
               fm.getFunctionContaining(A(handler&~1))) if in_image else False,
           "source_usable_for_actual_MMC_command_handler":True
        })
    subcmd["all_ten_descriptors_in_owner_rom"]=bool(
      len(subcmd["records"])==10 and
      all(x["name_sha256"] and x["handler_in_original_owner_image"]
          for x in subcmd["records"]))
else:
    subcmd["layout_error"]="MMC table pointer not inside owner P3; no assumed subcommand names"
(out/"V26-OWNER-MMC-SUBCOMMAND-TABLE.json").write_text(
    json.dumps(subcmd,indent=2)+"\n")
print("V26 MMC native subcommands",len(subcmd["records"]),
      "layout verified",subcmd["all_ten_descriptors_in_owner_rom"],flush=True)

# Source-level high P-code tracing of executable *recognized* helpers,
# independently of PseudoDisassembler recovery above.
roots={
  "native_MMC_cmd_handler":0x00104f10,
  "MMC_device_lookup_and_init":0x00104f88,
  "MMC_list_printing":0x00105620,
  "native_Uboot_cmd_invocation":0x001111e0,
  "actual_command_lookup":0x00110ccc,
  "native_command_wrapper":0x00110d2c,
  "mmc_partition":0x00105b50,
  "ext4_load":0x00102536,
  "fat_load":0x00102500,
  "source_script":0x00101cd0,
  "bootz":0x00101800,
  "autoboot":0x001006e4,
  "env_lookup":0x00108804,
  "env_update":0x0010854c,
  "env_persist":0x00108110,
  "storage_write":0x0011de50,
  "genuine_command_parser":0x00110484,
  "real_serial_output_UART_MMIO":0x0011f580,
  "original_serial_output_fallback":0x0011f2d6,
  "owner_output_formatting_to_MMIO":0x0010d14c,
  "native_MMC_command_subcommand_dispatch":0x001104f10
}
edges=defaultdict(set);incoming=defaultdict(set)
for entry,f in functions.items():
    it=listing.getInstructions(f.getBody(),True)
    while it.hasNext():
        ins=it.next()
        if not ins.getFlowType().isCall():continue
        for target in ins.getFlows():
            fn=fm.getFunctionContaining(target)
            if fn:
                where=int(fn.getEntryPoint().getOffset())
                if where in functions:
                    edges[entry].add(where);incoming[where].add(entry)
def reach(starts,graph):
    q=deque(starts);dist={s:0 for s in starts}
    while q:
        at=q.popleft()
        for candidate in graph.get(at,()):
            if candidate not in dist:
                dist[candidate]=dist[at]+1;q.append(candidate)
    return dist
focus={}
domains={}
for label,at in roots.items():
    fun=fm.getFunctionContaining(A(at))
    if not fun:continue
    initial=int(fun.getEntryPoint().getOffset())
    forward=reach([initial],edges);back=reach([initial],incoming)
    domains[label]={
       "entry":H(initial),"callee_reachable":len(forward),
       "caller_reachable":len(back),
       "nearest_callees":[{"entry":H(k),"depth":v}
                          for k,v in sorted(forward.items(),key=lambda x:x[1])[:120]],
       "nearest_callers":[{"entry":H(k),"depth":v}
                         for k,v in sorted(back.items(),key=lambda x:x[1])[:120]]
    }
    for graph in (forward,back):
        for key,d in graph.items():focus[key]=min(d,focus.get(key,99999))
def dependent(v,seen=None,budget=None):
    if budget is None:budget=[120]
    if budget[0]<=0:return {"kind":"budget"}
    budget[0]-=1
    if v is None:return {"kind":"null"}
    if seen is None:seen=set()
    row={"varnode":str(v)[:110]}
    if v.isConstant():
        row["kind"]="literal";row["value"]=H(v.getOffset());return row
    op=v.getDef()
    if not op:
        row["kind"]="unknown_input";return row
    tag=(str(v),str(op.getSeqnum()))
    if tag in seen:
        row["kind"]="cycle";return row
    row["kind"]=str(op.getMnemonic())
    row["pc"]=str(op.getSeqnum().getTarget())
    row["inputs"]=[dependent(op.getInput(i),seen|{tag},budget)
                   for i in range(min(op.getNumInputs(),6))]
    return row
dec=DecompInterface();dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(P):raise RuntimeError("Ghidra owner decompiler cannot open")
effects_path=out/"V26-ORIGINAL-BOOT-AND-MMC-PCODE.jsonl"
decompiled=0;fails=[]
try:
    with effects_path.open("w") as f:
        for i,entry in enumerate(sorted(
           [x for x,d in focus.items() if d<=3],
           key=lambda x:(focus[x],x))):
            fn=functions[entry]
            row={"entry":H(entry),"root_hops":focus[entry],
                 "name":str(fn.getName()),"effects":[]}
            try:
                res=dec.decompileFunction(fn,30,monitor)
                if not res or not res.decompileCompleted() or not res.getHighFunction():
                    raise RuntimeError(str(res.getErrorMessage()) if res else "no result")
                decompiled+=1
                pseudo_code=str(res.getDecompiledFunction().getC())
                pseudo_code=re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+",
                                   "inmusic-unlock-magic-[REDACTED]",pseudo_code)
                row["pseudocode"]=pseudo_code[:18000]
                instructions=res.getHighFunction().getPcodeOps()
                while instructions.hasNext():
                    op=instructions.next()
                    kind=str(op.getMnemonic())
                    if kind not in ("STORE","CALLIND","BRANCHIND","CALL","LOAD"):continue
                    site=int(op.getSeqnum().getTarget().getOffset())
                    index_target=1 if kind in ("STORE","LOAD") else 0
                    target=op.getInput(index_target) if op.getNumInputs()>index_target else None
                    effect={"at":H(site),"kind":kind,
                            "target_dependencies":dependent(target)}
                    if kind=="STORE" and op.getNumInputs()>2:
                        effect["stored_value_dependencies"]=dependent(op.getInput(2))
                    row["effects"].append(effect)
                count["pcode_decompiled"]+=1
                count["pcode_effects"]+=len(row["effects"])
            except Exception as exc:
                fails.append({"entry":H(entry),"error":str(exc)[:400]})
                row["error"]=str(exc)[:400]
            f.write(json.dumps(row)+"\n")
            if (i+1)%50==0:
                f.flush();print("V26 native boot/MMC high p-code",i+1,
                                "decompiled",decompiled,"effects",count["pcode_effects"],flush=True)
finally:
    dec.dispose()
summary={
  "schema":"mpfree-v26-raw-native-command-handlers-and-MMC-functions",
  "owner_SHA256":PIN,"owner_source_verified":True,
  "generated_utc":datetime.now(timezone.utc).isoformat(),
  "actual_registered_commands":86,
  "Ghidra_recognized_source_functions":len(functions),
  "saved_Ghidra_command_function_recognition":count["already_recognized_exact"],
  "previously_unrecognized_registered_handlers":count["previously_unrecognized"],
  "handlers_with_pseudodisassembly":count["handlers_with_pseudo"],
  "pseudo_instructions_decoded":count["pseudo_instructions"],
  "handler_disassembly_errors":count["handlers_with_errors"],
  "recognized_function_source_Pcode_decompiled":decompiled,
  "pcode_effects":count["pcode_effects"],
  "decompilation_errors":fails,"source_call_graph_roots":domains,
  "command_source_refs":report,
  "original_MMC_subcommand_registry":subcmd,
  "original_UART_MMIO_source":{
      "low_level_tx":"0x0011f580",
      "V25_fault_pc":"0x0011f592",
      "console_entry":"0x0010d14c",
      "uart_status_literal_cell":"0x0011f5a0",
      "uart_status_register_pointer":H(read32(0x0011f5a0)) if read32(0x0011f5a0) else None,
      "uart_data_literal_cell":"0x0011f5a4",
      "uart_data_register_pointer":H(read32(0x0011f5a4)) if read32(0x0011f5a4) else None,
      "original_UART_MMIO_never_executed_in_VM":True
  },
  "handler_assembly_output":outfile.name,
  "MMCandboot_pcode_output":effects_path.name,
  "provenance":{
    "raw_source_SHA_pinned":True,
    "saved_Ghidra_project_modified":False,
    "PseudoDisassembler_never_writes_program":True,
    "missing_handler_Ghidra_function_IS_NOT_invalid_command":True,
    "physical_SD_and_eMMC_ID_STILL_NOT_PROVEN":True
  }}
(out/"V26-OWNER-COMMAND-HANDLERS-SUMMARY.json").write_text(
    json.dumps(summary,indent=2)+"\n")
print("V26 completed original 86 command-handler Thumb recovery with",
      count["pseudo_instructions"],"instructions and",
      count["pcode_effects"],"typed operations",flush=True)
