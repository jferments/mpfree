#!/usr/bin/env python3
"""V5: turn exhaustive V3/V4 Ghidra coverage into interprocedural evidence.

Uses only previous PRIVATE JSON outputs. Does not open firmware binaries,
make assumptions about RAM globals, run target code or contact hardware.
Writes a PRIVATE question/evidence report and concise Markdown summary.
"""
import argparse
from collections import defaultdict, deque
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re

GRAPH_SHA={
 "p1":"6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f",
 "p3":"b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a",
}
# Anchors are verified addresses within *this exact* owner firmware image,
# not instructions for connecting to or altering the actual MPC.
PATHS={
 "p1":{
  "initial_sram_to_boot_loop":("0xff704060","0xff705350"),
  "boot_loop_to_media_load":("0xff705350","0xff7054a4"),
  "boot_loop_to_indirect_handoff":("0xff705350","0xff705310"),
 },
 "p3":{
  "board_recovery_to_environment":("0x00100fe4","0x001011e0"),
  "fastboot_dispatch_to_permission":("0x00122780","0x00122834"),
  "fastboot_dispatch_to_download":("0x00122780","0x00122980"),
  "fastboot_ram_boot_to_bootm":("0x001226b4","0x00101744"),
  "bootm_to_image_loader":("0x00101744","0x0010a6a4"),
  "image_classifier_to_fit_verify":("0x001017fc","0x0010f388"),
 }
}
KEYWORDS={
 "boot_selection":("dorecovery","bootcmd","fdtfile","boot_targets","bootdelay","mmc"),
 "rom_reset":("0xef08a53c","0x5242c300","0x5242c301","0x5242c309","reboot"),
 "fastboot_authorization":("fastboot_permission","DAT_00122928","DAT_0012284c"),
 "fastboot_ram":("mpf_fastboot_boot_done","mpf_fastboot_receive","mpf_fastboot_download"),
 "image_integrity":("mpf_legacy_header_crc","mpf_legacy_payload_crc",
                     "FUN_0010f388","FUN_0010f238","mpf_classify_image"),
 "memory_ownership":("stack","bootm_low","bootm_size","fdt_high","0x00800800"),
}
def read(root,name):
    f=root/name
    if f.is_symlink() or not f.is_file():return None
    return json.loads(f.read_text(encoding="utf-8"))
def get_all(stage,root):
    v3=read(root,stage+"-evidence-graph-v3.json")
    v4=read(root,stage+"-coverage-v4.json")
    if not v3 or not v4:raise RuntimeError("Missing verified V3/V4 reports for "+stage)
    sha=GRAPH_SHA[stage]
    if v3.get("schema")!="mpfree-evidence-graph-v3" or v3.get("sha256")!=sha:
        raise RuntimeError("Wrong V3 graph source")
    if v4.get("schema")!="mpfree-recognized-function-coverage-v4" or v4.get("source_payload_sha256")!=sha:
        raise RuntimeError("Wrong V4 coverage source")
    graph=v3["all_known_function_graph"]
    detailed={**v3["detailed_functions"],**v4["coverage"]}
    errors={k: f["decompile_error"] for k,f in detailed.items() if f.get("decompile_error")}
    return graph,detailed,errors
def shortest(graph,start,goal):
    if start not in graph or goal not in graph:
        return None
    q=deque([start]);prev={start:None}
    while q:
        current=q.popleft()
        if current==goal:
            path=[];c=goal
            while c is not None:
                path.append(c);c=prev[c]
            return list(reversed(path))
        for candidate in graph[current].get("calls",[]):
            if candidate not in prev and candidate in graph:
                prev[candidate]=current;q.append(candidate)
    return None
def assess(stage,root):
    graph,code,errors=get_all(stage,root)
    callers=defaultdict(set)
    for addr,node in graph.items():
        for target in node.get("calls",[]):callers[target].add(addr)
    result={
      "stage":stage,
      "recognized_functions":len(graph),
      "detailed_function_entries":len(code),
      "decompile_error_count":len(errors),"decompile_errors":errors,
      "direct_call_edges":sum(len(x.get("calls",[])) for x in graph.values()),
      "computed_call_count":sum(len(x.get("computed_calls",[])) for x in graph.values()),
      "unresolved_direct_call_destinations":sorted({
        target for node in graph.values() for target in node.get("calls",[])
        if target not in graph})[:160],
      "paths":{},"categorized_global_interactions":{},"central_indirect_functions":[],
      "limitations":[
       "Shortest paths include Ghidra direct CALL edges only, not branch or computed-call paths.",
       "No direct path does NOT imply the runtime path is impossible.",
       "Keyword writers are heuristics over decompiler pseudocode, not proven executed writes.",
       "Some decompilations fail; recognized-function coverage is not full instruction coverage.",
      ]
    }
    for name,(source,dest) in PATHS[stage].items():
        route=shortest(graph,source,dest)
        result["paths"][name]={
          "from":source,"to":dest,
          "direct_call_path_found":route is not None,
          "path":route or [],
          "path_names":[graph[a]["name"] for a in route] if route else [],
          "confidence":"static Ghidra direct-call chain, NOT runtime-execution proof" if route else
                       "unresolved: may use indirect calls, jumps, or alternate entry"}
    ranked=sorted(
      [(len(node.get("computed_calls",[])),len(callers.get(addr,set())),addr,node["name"])
       for addr,node in graph.items() if node.get("computed_calls")],
      reverse=True)
    result["central_indirect_functions"]=[
      {"address":addr,"name":name,"computed_calls":n,"direct_callers":fans,
       "call_sites":graph[addr].get("computed_calls",[])[:20]}
      for n,fans,addr,name in ranked[:35]]
    for cat,needles in KEYWORDS.items():
        matches=[]
        for address,fn in code.items():
            pseudocode=fn.get("pseudocode","")
            if not pseudocode:continue
            terms=[t for t in needles if t.lower() in pseudocode.lower()]
            if not terms:continue
            # Truncated source excerpts remain inside the private JSON output
            # rather than being committed. We only export classification.
            patterns=re.findall(r"\*\s*(?:PTR_)?DAT_[0-9a-fA-F]+(?:\s*=\s*[^;]+)?",
                                pseudocode)
            matches.append({
                "address":address,"name":fn.get("name"),
                "matched_terms":terms,
                "has_indirect_call":bool(graph.get(address,{}).get("computed_calls")),
                "callers":sorted(callers.get(address,set()))[:30],
                "direct_callees":graph.get(address,{}).get("calls",[])[:40],
                "candidate_pointer_derefs":patterns[:15],
            })
        result["categorized_global_interactions"][cat]=matches[:120]
    return result
def interpret_v4(root):
    r=read(root,"firmware-3.10-bootchain-v4-compare.json") or {}
    data={"available":bool(r),"script_normalized_equal":None,
          "board_changes":{},"program_presence_and_hashes":{},
          "bootloader_sig_control_flow":{}}
    if not r:return data
    data["script_normalized_equal"]=r.get("boot_script",{}).get("normalized_commands_equal")
    for name,entry in r.get("board_dtbs",{}).items():
        data["board_changes"][name]=entry.get("boot_node_diff",{}).get("changed_boot_nodes",{})
    for filename,versions in r.get("elfs",{}).items():
        data["program_presence_and_hashes"][filename]={
          ver:{"present":e.get("present"),"sha256":e.get("sha256"),
               "bytes":e.get("bytes")} for ver,e in versions.items()}
    utility=r.get("elfs",{}).get("/usr/bin/az01-bootloader-sig",{}).get("firmware310",{})
    lines=utility.get("small_elf_complete_text",{}).get("lines",[])
    events=[]
    for row in lines:
        # Retain only stable high-level instruction *classifications*;
        # don't copy proprietary disassembly into public documentation.
        if any(s in row for s in ("memcmp@plt","errx@plt",
                                   "RC4_set_key@plt","RC4@plt",
                                   "SHA256_Update@plt","SHA256_Final@plt")):
            m=re.search(r"^\s*([a-fA-F0-9]+):",row)
            if m:
                label=next((x for x in ("memcmp","errx","RC4_set_key","RC4",
                                          "SHA256_Update","SHA256_Final")
                            if x+"@plt" in row),"other")
                if "\tbl\t" in row or "\tblx\t" in row:
                    events.append({"at":"0x"+m.group(1),"call":label})
    data["bootloader_sig_control_flow"]={
        "sha256_import_present":any(x["call"]=="SHA256_Update" for x in events),
        "compares_digests":any(x["call"]=="memcmp" for x in events),
        "exit_on_error_api_present":any(x["call"]=="errx" for x in events),
        "rc4_api_present":any(x["call"].startswith("RC4") for x in events),
        "call_events":events[:90],
        "interpretation":"The CLI contains a digest comparison and fatal error calls. This does not prove it is run at boot, or establish BootROM signature enforcement.",
    }
    return data
def report(root):
    sources={s:assess(s,root) for s in ("p1","p3")}
    return {"schema":"mpfree-v5-crossstage-path-and-security-v1",
            "generated_utc":datetime.now(timezone.utc).isoformat(),
            "source":"Verified owner-only V3/V4 offline forensic JSON reports",
            "bootloader_stages":sources,
            "firmware_security":interpret_v4(root),
            "hardware_proof_gates":[
             "Genuine MaskROM USB enumeration, including vendor/device identity.",
             "Non-destructive independent eMMC readback and restore verification.",
             "Owner-built SPL acceptance and DDR initialization on physical board.",
             "Run-time U-Boot permission/default environment state.",
             "Measured relocation/stack/download/kernel memory layout.",
             "SD-first boot selection without writing the on-device eMMC environment.",
            ],
            "private":"Contains Ghidra-derived addresses and summarized proprietary code; do not commit."}
def summarize(r):
    lines=["# MPFree V5 cross-stage path and security synthesis","",
           "**Offline static evidence only; no device contact or hardware proof.**",""]
    for stage,entry in r["bootloader_stages"].items():
        lines.extend(["## "+stage.upper()+" function coverage","",
          "- Recognized: %d, detailed entries: %d; decompilation errors: %d."%
             (entry["recognized_functions"],entry["detailed_function_entries"],
              entry["decompile_error_count"]),
          "- Direct call edges: %d; computed call sites: %d."%
             (entry["direct_call_edges"],entry["computed_call_count"]),""])
        for key,path in entry["paths"].items():
            lines.append("- %s: %s"%(key,
                " -> ".join(path["path"]) if path["direct_call_path_found"]
                else "no complete direct-call path recovered"))
        lines.append("")
    f=r["firmware_security"]
    lines.extend(["## Firmware 3.10 userland evidence","",
                  "- Normalized 3.6 / 3.10 boot scripts equal: %s."%
                    f.get("script_normalized_equal"),""])
    for name,nodes in f["board_changes"].items():
        lines.append("- %s: %d changed boot-mode nodes."%(name,len(nodes)))
    lines.extend(["","## What cannot be inferred","",
                  "- A matching static call graph does not establish live executable reachability.",
                  "- A userspace bootloader hash-check utility is not silicon secure-boot enforcement.",
                  "- No MaskROM entry, unsigned-SPL acceptance or independent flash recovery is proven.",
                  "- An absent direct-call chain can be bridged by indirect calls or jump tables.",
                  ""])
    return "\n".join(lines)
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("report_dir",type=Path)
    args=p.parse_args()
    root=args.report_dir.expanduser()
    if root.is_symlink() or not root.is_dir() or not root.name.startswith("bootchain-wide-"):
        p.error("Require real bootchain-wide-* report folder")
    data=report(root)
    old=os.umask(0o077)
    try:
        (root/"bootchain-v5-synthesis.json").write_text(json.dumps(data,indent=2)+"\n",
                                                         encoding="utf-8")
        (root/"bootchain-v5-summary.md").write_text(summarize(data),encoding="utf-8")
    finally:os.umask(old)
    print("MPFree V5 evidence synthesis complete:",root)
if __name__=="__main__":main()
