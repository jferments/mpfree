# @runtime PyGhidra
# @category MPFree
# Per-ELF owner-private decompilation in an EPHEMERAL Ghidra import project.
# The launcher validates 3.10 rootfs ext4 & ELF magic before invoking.
# Never load the MPC, flash partitions, program OTP, or execute the ELF.
import json
import os
import re
from pathlib import Path
from datetime import datetime, timezone
from ghidra.app.decompiler import DecompInterface

args=list(getScriptArgs())
if len(args)!=3:
    raise RuntimeError("Expected private_report_path source_label verified_sha256")
dest=Path(str(args[0]));label=str(args[1]);sha=str(args[2])
if not dest.is_absolute() or not re.fullmatch(r"[0-9a-f]{64}",sha):
    raise RuntimeError("Invalid report destination or preverified source SHA")
p=currentProgram
if p is None or "ELF" not in str(p.getExecutableFormat()).upper():
    raise RuntimeError("Expected auto-imported ELF executable")
fm=p.getFunctionManager()
refs=p.getReferenceManager()
listing=p.getListing()
mem=p.getMemory()
interesting=re.compile(
  r"bootloader|partition|mmcblk|rootfs|update|sig|hash|sha256|rc4|"
  r"verity|trustpoint|loadpin|reboot|mode|otp|efuse|fuse|"
  r"/dev/|/sys/|/etc/|service|firmware|recovery|mismatch",
  re.I)
sensitive=re.compile(r"password|secret|unlock|private.?key|token|"
                     r"credential|serialno|ethaddr",re.I)
def redacted(s):
    if sensitive.search(s):return "[PRIVATE LITERAL REDACTED]"
    return s[:220]
functions=[]
it=fm.getFunctions(True)
while it.hasNext():functions.append(it.next())
strings=[];seen=set()
it=listing.getDefinedData(True)
while it.hasNext() and len(strings)<1200:
    item=it.next()
    try:
        value=item.getValue()
        if value is None:continue
        literal=str(value)
        if len(literal)<5 or len(literal)>3000:continue
        if not interesting.search(literal):continue
        addr=str(item.getAddress())
        if addr in seen:continue
        seen.add(addr)
        xrefs=[]
        for ref in refs.getReferencesTo(item.getAddress()):
            fn=fm.getFunctionContaining(ref.getFromAddress())
            xrefs.append({
              "from":str(ref.getFromAddress()),
              "caller":str(fn.getEntryPoint()) if fn else None,
              "kind":str(ref.getReferenceType())})
            if len(xrefs)>=70:break
        strings.append({"address":addr,"text":redacted(literal),
                        "references":xrefs})
    except Exception:continue
# Rather than trusting symbol names, prioritize every function with xrefs
# to partitions, hashes and boot-loader utility diagnostics.
priority={}
for item in strings:
    for ref in item["references"]:
        if not ref.get("caller"):continue
        k=ref["caller"];priority[k]=priority.get(k,0)+8
for fn in functions:
    key=str(fn.getEntryPoint());name=str(fn.getName())
    if interesting.search(name):priority[key]=priority.get(key,0)+18
# Also include ALL functions in small helpers to reconstruct full startup,
# main, hash compare, error paths even when no string xref exists.
ordered=sorted(functions,key=lambda fn:(
    -priority.get(str(fn.getEntryPoint()),0),
    int(fn.getEntryPoint().getOffset())))
if len(functions)<=160:ordered=sorted(functions,key=lambda fn:int(fn.getEntryPoint().getOffset()))
decompiler=DecompInterface()
decompiler.toggleCCode(True)
decompiler.setSimplificationStyle("decompile")
if not decompiler.openProgram(p):raise RuntimeError("Could not open imported ELF decompiler")
out={"schema":"mpfree-private-imported-elf-ghidra-v5",
    "date_utc":datetime.now(timezone.utc).isoformat(),
    "source_label":label,"source_sha256_verified_by_launcher":sha,
    "format":str(p.getExecutableFormat()),
    "language":str(p.getLanguageID()),
    "recognized_functions":len(functions),
    "selected_function_limit":min(len(ordered),220),
    "selected_strings":strings,
    "functions":{},
    "limits":[
       "Ghidra analysis only; imported ELF is not executed on MPC or workstation.",
       "Symbol, decompiler, GOT and ARM/Thumb indirect call resolution may be wrong.",
       "CLI presence does not imply execution at boot or mandatory security policy.",
       "Source SHA was verified by the workstation runner, not read back from device.",
       "PRIVATE decompilation; never commit this JSON to public GitHub.",
    ]}
try:
    for fn in ordered[:220]:
        addr=str(fn.getEntryPoint())
        details={"name":str(fn.getName()),
                 "address":addr,"body_size":int(fn.getBody().getNumAddresses()),
                 "incoming_callers":[],"outgoing_calls":[]}
        for ref in refs.getReferencesTo(fn.getEntryPoint()):
            if ref.getReferenceType().isCall():
                caller=fm.getFunctionContaining(ref.getFromAddress())
                details["incoming_callers"].append({
                    "site":str(ref.getFromAddress()),
                    "caller":str(caller.getEntryPoint()) if caller else None})
                if len(details["incoming_callers"])>=70:break
        inst=listing.getInstructions(fn.getBody(),True)
        while inst.hasNext():
            ins=inst.next()
            if ins.getFlowType().isCall():
                details["outgoing_calls"].append({
                    "at":str(ins.getAddress()),
                    "targets":[str(x) for x in ins.getFlows()]})
                if len(details["outgoing_calls"])>=120:break
        try:
            decomp=decompiler.decompileFunction(fn,70,monitor)
            if decomp and decomp.decompileCompleted() and decomp.getDecompiledFunction():
                code=str(decomp.getDecompiledFunction().getC())
                code=re.sub(r'"[^"\n]{80,}"','"[LONG STRING REDACTED]"',code)
                code=re.sub(r'"[^"\n]*(?:password|secret|unlock|token|private.key|serialno)[^"\n]*"',
                            '"[PRIVATE LITERAL REDACTED]"',code,flags=re.I)
                details["pseudocode"]=code[:16000]
                details["pseudocode_truncated"]=len(code)>16000
            else:
                details["decompile_error"]=str(decomp.getErrorMessage()) if decomp else "no result"
        except Exception as e:details["decompile_error"]=str(e)
        out["functions"][addr]=details
finally:decompiler.dispose()
dest.parent.mkdir(parents=True,exist_ok=True)
umask=os.umask(0o077)
try:dest.write_text(json.dumps(out,indent=2)+"\n",encoding="utf-8")
finally:os.umask(umask)
print("MPFree V5 ELF IMPORT:",label,"functions",len(functions),
      "decompiled",len(out["functions"]),"strings",len(strings),"output",str(dest))
