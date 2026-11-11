#!/usr/bin/env python3
"""Summarize the owner-private MPFree boot-chain audit with evidence boundaries.

Reads only completed JSON reports in an existing bootchain-wide-* folder,
never opens hardware, extracted firmware, or project files. Emits a small
evidence-ledger.json + evidence-summary.md INSIDE the same private report
directory before the single-ZIP packager runs.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path

SOURCES={
 "p1_graph":"p1-evidence-graph-v3.json",
 "p3_graph":"p3-evidence-graph-v3.json",
 "p1_coverage":"p1-coverage-v4.json",
 "p3_coverage":"p3-coverage-v4.json",
 "boot_artifacts":"storage-env-boot-dtb.json",
 "cross_stage_fdt":"p1-p3-embedded-fdt-crossstage.json",
 "firmware310":"firmware-3.10-boot-compare.json",
 "firmware310_v4":"firmware-3.10-bootchain-v4-compare.json",
 "rootfs_locator":"firmware-3.10-rootfs-locator.json",
}
def safe_read(root,filename):
    path=root/filename
    if path.is_symlink() or not path.is_file():
        return {"available":False,"reason":"file missing"}
    try:
        contents=json.loads(path.read_text(encoding="utf-8"))
        return {"available":True,"data":contents}
    except Exception as e:
        return {"available":False,"reason":type(e).__name__}
def make(root):
    inputs={k:safe_read(root,f) for k,f in SOURCES.items()}
    def get(name):
        return inputs.get(name,{}).get("data",{})
    findings=[]
    def add(question,status,evidence,source,qualification):
        findings.append({"question":question,"status":status,"evidence":evidence,
                         "source":source,"qualification":qualification})
    a=get("boot_artifacts")
    env=a.get("p2",{})
    if env.get("crc_valid_for_standard_layout"):
        add("P2 environment integrity","offline data validated",
            "Saved environment CRC32 matches standard-layout calculated CRC32.",
            SOURCES["boot_artifacts"],"Not proof U-Boot reads this environment at runtime.")
    defaults=a.get("compiled_uboot_environment",{}).get("saved_comparison",{})
    if defaults:
        matches=sum(v.get("same_as_saved") is True for v in defaults.values())
        add("U-Boot compiled defaults","candidate compiled environment",
            "%s/%s candidate variables match saved P2."%(matches,len(defaults)),
            SOURCES["boot_artifacts"],"Fallback behavior and runtime values are unproven.")
    fdt=get("cross_stage_fdt")
    spl=fdt.get("spl_stage_fdt",[])
    uboot=fdt.get("uboot_stage_fdt",[])
    if spl and uboot:
        x=spl[0].get("selected_properties",{})
        y=uboot[0].get("selected_properties",{})
        names=("rockchip,pctl-timing","rockchip,phy-timing","rockchip,sdram-params")
        same=sum(bool(x.get(n)) and x.get(n)==y.get(n) for n in names)
        add("SPL/U-Boot SDRAM parameters","static board config",
            "Both stages have embedded FDTs; %d/%d DDR parameter arrays match."%(same,len(names)),
            SOURCES["cross_stage_fdt"],
            "No evidence of successful initialization by owner-built SPL.")
    compare=get("firmware310_v4")
    if compare:
        script=compare.get("boot_script",{})
        status=("same executable script" if script.get("normalized_commands_equal")
                else "different or unavailable executable script")
        add("3.6/3.10 executable boot script",status,
            "Normalized multi-component U-Boot boot-script contents %s."%
                ("match" if script.get("normalized_commands_equal") else "do not match or missing"),
            SOURCES["firmware310_v4"],
            "Header CRC/timestamp may differ even when script commands match.")
        for board,item in compare.get("board_dtbs",{}).items():
            n=item.get("boot_node_diff",{}).get("nodes_changed")
            if n is not None:
                add("3.10 boot DTB "+board,"offline property diff",
                    "%d changed boot-related node/property collections."%n,
                    SOURCES["firmware310_v4"],
                    "Phandle numbering, kernel binding support and runtime behavior need review.")
        elfs=compare.get("elfs",{})
        for name in ("/usr/bin/az01-bootloader-sig",
                     "/usr/lib/libaz01-prog.so.1.3",
                     "/usr/sbin/az0x-loadpin-trustpoints"):
            versions=elfs.get(name,{})
            add("3.6/3.10 "+name,"offline executable inventory",
                "factory present=%s; 3.10 present=%s; ARM ELF symbols/disassembly in source report."%
                    (versions.get("factory",{}).get("present"),
                     versions.get("firmware310",{}).get("present")),
                SOURCES["firmware310_v4"],
                "Imported crypto APIs and partition strings are not silicon enforcement.")
    for stage in ("p1","p3"):
        graph=get(stage+"_graph")
        cov=get(stage+"_coverage")
        total=len(graph.get("all_known_function_graph",{}))
        prior=len(graph.get("detailed_functions",{}))
        further=cov.get("newly_detailed",0)
        if total:
            status=("recognized-function coverage complete" if (prior+further)>=total
                    else "recognized-function coverage partial")
            add(stage.upper()+" Ghidra function analysis",status,
                "%d recognized functions, %d V3 detailed, %d V4 added, %d remaining."%
                     (total,prior,further,max(total-prior-further,0)),
                (SOURCES[stage+"_graph"]+" and "+SOURCES[stage+"_coverage"]),
                "Full Ghidra recognized-function coverage is NOT complete ARM/Thumb code or indirect execution coverage.")
    hard=[
      ("Silicon MaskROM entry","requires hardware","No confirmed Rockchip ROM-mode enumeration on this unit."),
      ("Unsigned owner-built SPL acceptance","requires hardware","Static RK32 image format cannot prove BootROM acceptance."),
      ("Independent complete eMMC recovery","requires hardware","No proven independent eMMC readback/restore from genuine ROM mode."),
      ("Secure boot eFuse policy","requires hardware","Neither embedded DTBs nor absence of direct eFuse xrefs proves state."),
      ("Fastboot runtime permission","requires hardware","Static U-Boot global references cannot establish initialized authorization."),
      ("Safe direct-boot memory map","unresolved","Need verified relocated U-Boot/stack/FDT/download bounds and subsequent owner-controlled test."),
      ("SD-first boot without eMMC writes","unresolved","Environment mmc0 mmc1 gives fallback only; no verified transient user selection."),
    ]
    for q,st,why in hard:
        add(q,st,why,"physical board or deeper bootloader control-flow proof",
            "Do not modify boot partitions or fuses as part of this offline audit.")
    stages={k:{"file":filename,"available":inputs[k]["available"],
               "reason":inputs[k].get("reason")} for k,filename in SOURCES.items()}
    ledger={"schema":"mpfree-bootchain-evidence-ledger-v1",
            "generated_utc":datetime.now(timezone.utc).isoformat(),
            "status":"Offline observation, static inference, and hardware proof separately labeled.",
            "passes":stages,"findings":findings,
            "open_questions_document":"docs/hardware/bootchain-open-questions-2026-11-11.md",
            "safety":"All reports private; no MPC communication, flashing, ROM entry, fuse programming or firmware execution."}
    return ledger
def render(a):
    out=["# MPFree private boot-chain evidence ledger",
         "",
         "**Static analysis only.** Treat hardware operation and recovery as unproven.",
         "",
         "## Audit coverage",
         ""]
    for key,row in a["passes"].items():
        out.append("- %s: %s (%s)"%(
           key,"present" if row["available"] else "missing or failed",row["file"]))
    out.extend(["","## Question evidence and limits",""])
    out.append("| Question | Status | Evidence | Limitation |")
    out.append("| --- | --- | --- | --- |")
    for f in a["findings"]:
        clean=lambda s:str(s).replace("|","/").replace("\n"," ")
        out.append("| %s | %s | %s | %s |"%tuple(clean(f[k]) for k in
                  ("question","status","evidence","qualification")))
    out.extend(["","## More detailed reports","",
        "For exact function code, inspect the JSON files inside this ZIP.",
        "The full question inventory is in the public source-only research document;",
        "never commit this report or proprietary disassembly to public GitHub.",
        ""])
    return "\n".join(out)
def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("report_dir",type=Path)
    args=ap.parse_args()
    root=args.report_dir.expanduser()
    if not root.is_dir() or root.is_symlink() or not root.name.startswith("bootchain-wide-"):
        ap.error("Require private bootchain-wide-* report directory")
    data=make(root)
    old=os.umask(0o077)
    try:
        (root/"evidence-ledger.json").write_text(json.dumps(data,indent=2)+"\n",encoding="utf-8")
        (root/"evidence-summary.md").write_text(render(data),encoding="utf-8")
    finally:os.umask(old)
    print("Evidence ledger:",len(data["findings"]),"findings; %d/%d passes present"%(
        sum(x["available"] for x in data["passes"].values()),len(data["passes"])))
if __name__=="__main__":main()
