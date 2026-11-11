// MPFreeFinishBootChain.java
// @category MPFree
//
// Native Java GhidraScript. Works under Ghidra 12.2 standard analyzeHeadless:
// NO PyGhidra, Jython extension, Python setup, or external modules needed.
// Usage: -preScript MPFreeFinishBootChain.java pre
//        -postScript MPFreeFinishBootChain.java post
//
// Only modifies the local Ghidra project; never contacts MPC/eMMC/USB.
import java.io.*;
import java.math.BigInteger;
import java.security.MessageDigest;
import java.text.SimpleDateFormat;
import java.util.*;
import ghidra.app.script.GhidraScript;
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.lang.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.symbol.*;

public class MPFreeFinishBootChain extends GhidraScript {
    private static final long BASE=0x100000L, SIZE=363649L;
    private static final String SHA="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a";
    private static final String[][] SEEDS={
        {"bootm_command","00101744","Y"},
        {"bootm_image_loading","0010a6a4","Y"},
        {"classify_image","0010e390","Y"},
        {"legacy_header_crc","0010e000","Y"},
        {"legacy_payload_crc","0010e02a","Y"},
        {"legacy_print_metadata","0010e1cc","Y"},
        {"fdt_header_checker","00127a1c","Y"},
        {"run_command_candidate","00110484","Y"},
        {"reset_cpu_candidate","0011622a","N"},
        {"usb_exit_state_read","00121dc4","Y"},
        {"usb_exit_state_set","00121dd0","Y"},
        {"usb_exit_state_clear","00121ddc","Y"},
        {"fastboot_continue_done","001226b0","Y"},
        {"fastboot_boot_done","001226b4","Y"},
        {"fastboot_response","00122734","Y"},
        {"fastboot_prefix_cmp","00122710","Y"},
        {"fastboot_dispatch","00122780","Y"},
        {"fastboot_permission","00122834","Y"},
        {"fastboot_oem","00122854","N"},
        {"fastboot_continue","00122948","Y"},
        {"fastboot_boot","00122964","Y"},
        {"fastboot_download","00122980","Y"},
        {"fastboot_receive","00122a10","Y"},
        {"fastboot_erase","00122bf8","N"},
        {"fastboot_flash","00122c5c","N"},
        {"fastboot_reboot","00122cf8","N"},
        {"fastboot_getvar","00122d38","Y"},
        {"memcpy_candidate","0012dda2","N"},
        {"crc_header_candidate","0012da74","N"},
        {"crc_data_candidate","0012da80","N"},
        {"image_loader_candidate","0010d14c","N"},
    };
    private static final String[][] GLOBALS={
        {"fastboot_permission", "0014d690"},
        {"fastboot_received", "0014d680"},
        {"fastboot_requested", "0014d684"},
        {"fastboot_exit", "0014d658"},
        {"download_addr", "00143e34"},
        {"command_table", "0012f2b0"}
    };
    private static final String[][] TABLE={
        {"reboot","00122cf9"}, {"getvar:","00122d39"},
        {"download:","00122981"}, {"boot","00122965"},
        {"continue","00122949"}, {"flash","00122c5d"},
        {"erase","00122bf9"}, {"oem","00122855"}
    };
    private static final String[][] EXPECTED_CALLS={
        {"001226de","00101744","fastboot response completion -> bootm"},
        {"0010a758","0010e390","bootm -> format identifier"},
        {"0010a7a6","0010e000","legacy uImage -> header CRC"},
        {"0010a7d0","0010e02a","legacy uImage -> data CRC"},
        {"0012298c","00122834","download -> permission check"},
        {"00122c00","00122834","erase -> permission check"},
        {"00122c64","00122834","flash -> permission check"},
        {"0010e39a","00127a1c","FDT header classifier"}
    };

    private Memory memory;
    private AddressSpace space;
    private ArrayList<String> result = new ArrayList<>();

    private Address addr(long a) { return space.getAddress(a); }
    private long hx(String s) { return Long.parseLong(s,16); }
    private int ub(long a) throws Exception { return memory.getByte(addr(a)) & 0xff; }
    private long le32(long a) throws Exception {
        return ((long)ub(a)) | ((long)ub(a+1)<<8) |
               ((long)ub(a+2)<<16) | ((long)ub(a+3)<<24);
    }
    private void line(String text){ result.add(text); }
    private void info(String text){ println("MPFree: "+text); }

    @Override
    protected void run() throws Exception {
        String[] args = getScriptArgs();
        String phase = args.length==0 ? "gui" : args[0];
        if (!phase.equals("pre") && !phase.equals("post") && !phase.equals("gui")) {
            throw new IllegalArgumentException("phase must be pre, post or gui");
        }
        if(currentProgram == null) throw new IllegalStateException("No open U-Boot program");
        if(!currentProgram.getLanguageID().toString().equals("ARM:LE:32:v7")) {
            throw new IllegalStateException("Wrong processor "+currentProgram.getLanguageID());
        }
        memory=currentProgram.getMemory();
        space=currentProgram.getAddressFactory().getDefaultAddressSpace();

        MemoryBlock target = null;
        int count = 0;
        for (MemoryBlock mb: memory.getBlocks()) {
            if(mb.isInitialized() && mb.getSize()==SIZE) { count++; target=mb; }
        }
        if(count!=1) throw new IllegalStateException(
            "Expected one 363649-byte raw p3 payload block; got "+count);
        long start=target.getStart().getOffset();
        if(start!=0 && start!=BASE)
            throw new IllegalStateException("Unexpected image start 0x"+Long.toHexString(start));
        if(start==0 && currentProgram.getImageBase().getOffset()!=0)
            throw new IllegalStateException("Raw block starts at zero but image base is not zero");

        MessageDigest md=MessageDigest.getInstance("SHA-256");
        byte[] buffer=new byte[32768];
        for(long pos=0;pos<SIZE;){
            int n=(int)Math.min(buffer.length,SIZE-pos);
            int r=memory.getBytes(addr(start+pos),buffer,0,n);
            if(r!=n) throw new IOException("Short read at "+Long.toHexString(pos));
            md.update(buffer,0,n);
            pos+=n;
        }
        StringBuilder hash=new StringBuilder();
        for(byte value:md.digest()) hash.append(String.format("%02x",value & 0xff));
        if(!SHA.equals(hash.toString())) throw new IllegalStateException(
            "SHA256 mismatch "+hash+"; stopping before any edits");

        if(le32(start+0x2f2b0)!=0x0013f08bL ||
           le32(start+0x2f2b4)!=0x00122cf9L ||
           le32(start+0x2f2cc)!=0x00122965L) {
            throw new IllegalStateException("Fastboot table mismatch; wrong program?");
        }

        if(start==0) {
            int tx=currentProgram.startTransaction("MPFree rebase original p3 to 0x00100000");
            boolean success=false;
            try {
                currentProgram.setImageBase(addr(BASE),true);
                if(memory.getBlock(addr(BASE))==null)
                    throw new IllegalStateException("Rebase failed");
                success=true;
            } finally {
                currentProgram.endTransaction(tx,success);
            }
            info("Rebased local Ghidra project from 0 to 0x00100000");
        } else {
            info("Base 0x00100000 verified (no rebase needed)");
        }

        if(phase.equals("pre") || phase.equals("gui")) {
            seedThumb();
            if(phase.equals("pre")) {
                info("PreScript complete, Ghidra will now auto-analyze");
                return;
            }
            analyzeAll(currentProgram);
        }
        exportReport(hash.toString());
    }

    private void seedThumb() throws Exception {
        Register t=currentProgram.getProgramContext().getRegister("TMode");
        if(t==null) throw new IllegalStateException("Missing ARM TMode context register");
        RegisterValue thumb=new RegisterValue(t,BigInteger.ONE);
        FunctionManager fm=currentProgram.getFunctionManager();
        Listing listing=currentProgram.getListing();
        info("Seeding Thumb instruction entrypoints");
        for(String[] seed:SEEDS){
            long target=hx(seed[1]);
            Address a=addr(target);
            if(!memory.contains(a)) continue;
            if(listing.getInstructionAt(a)==null) {
                try {
                    DisassembleCommand dis=new DisassembleCommand(
                        new AddressSet(a,a),null,true);
                    dis.setInitialContext(thumb);
                    dis.applyTo(currentProgram,monitor);
                } catch(Exception e) {
                    info("Disassembly seed "+seed[0]+" failed: "+e);
                }
            }
            if(listing.getInstructionAt(a)!=null &&
               fm.getFunctionAt(a)==null &&
               fm.getFunctionContaining(a)==null) {
                try {
                    createFunction(a,"mpf_"+seed[0]);
                } catch(Exception e) {
                    info("Function creation "+seed[0]+" failed: "+e);
                }
            }
        }
    }

    private void exportReport(String hash) throws Exception {
        result.clear();
        line("MPFree p3 final Ghidra analysis (native Java, Ghidra 12.2)");
        line("SHA256 "+hash);
        line("Image base "+currentProgram.getImageBase());
        line("Payload block base "+memory.getBlock(addr(BASE)).getStart());
        line("Program "+currentProgram.getName());
        line("Do NOT confuse raw on-disk bytes of BSS region with runtime state.");
        line("No MPC connection or firmware modifications performed.");
        line("");

        line("=== EIGHT FASTBOOT DISPATCH ENTRIES ===");
        for(int i=0;i<TABLE.length;i++){
            long row=BASE+0x2f2b0+8L*i;
            long sp=le32(row), cb=le32(row+4);
            StringBuilder value=new StringBuilder();
            if(sp>=BASE && sp<BASE+SIZE) {
                for(int j=0;j<32 && sp+j<BASE+SIZE;j++){
                    int cc=ub(sp+j);if(cc==0) break;
                    if(cc<32 || cc>126) { value.append('?');break;}
                    value.append((char)cc);
                }
            }
            boolean ok=value.toString().equals(TABLE[i][0]) &&
                cb==hx(TABLE[i][1]);
            line(String.format(Locale.ROOT,
                "%02d %-11s row=%08x cb=%08x Thumb=%s valid=%s",
                i,TABLE[i][0],row,cb & ~1L,(cb & 1)!=0,ok));
        }

        line("");
        line("=== GLOBAL ADDRESSES: xrefs; file values ARE NOT runtime state ===");
        ReferenceManager refs=currentProgram.getReferenceManager();
        for(String[] glob:GLOBALS){
            long va=hx(glob[1]);
            List<String> locations=new ArrayList<>();
            ReferenceIterator it=refs.getReferencesTo(addr(va));
            while(it.hasNext() && locations.size()<100){
                Reference r=it.next();
                locations.add(r.getFromAddress()+":"+r.getReferenceType());
            }
            line(String.format(Locale.ROOT,
                "%-23s addr=0x%08x raw=0x%08x xrefs=%s",
                glob[0],va,le32(va),locations));
        }

        line("");
        line("=== VERIFIED Ghidra CALL EDGES ===");
        Listing listing=currentProgram.getListing();
        for(String[] edge:EXPECTED_CALLS){
            long at=hx(edge[0]), to=hx(edge[1]);
            Instruction inst=listing.getInstructionAt(addr(at));
            boolean match=false;
            ArrayList<String> flows=new ArrayList<>();
            if(inst!=null){
                for(Address a:inst.getFlows()){
                    flows.add(a.toString());
                    if(a.getOffset()==to)match=true;
                }
            }
            line(String.format("%s: %s -> %s: %s (actual=%s)",
                edge[2],edge[0],edge[1],match ? "MATCH":"UNVERIFIED",flows));
        }

        line("");
        line("=== FOCUSED FUNCTIONS AND PSEUDOCODE ===");
        FunctionManager fm=currentProgram.getFunctionManager();
        DecompInterface dc=new DecompInterface();
        dc.toggleCCode(true);
        dc.setSimplificationStyle("decompile");
        boolean opened=dc.openProgram(currentProgram);
        line("Decompiler initialized="+opened);
        try{
            for(String[] seed:SEEDS){
                long va=hx(seed[1]);
                Function fn=fm.getFunctionAt(addr(va));
                line("");
                line("FUNCTION "+seed[0]+" @ "+seed[1]);
                if(fn==null){
                    Function enclosing=fm.getFunctionContaining(addr(va));
                    line("Not recognized as a function; enclosing="+enclosing);
                    continue;
                }
                line("name="+fn.getName()+" size="+fn.getBody().getNumAddresses());
                ArrayList<String> incoming=new ArrayList<>();
                ReferenceIterator iter=refs.getReferencesTo(addr(va));
                while(iter.hasNext() && incoming.size()<100){
                    Reference r=iter.next();
                    incoming.add(r.getFromAddress()+":"+r.getReferenceType());
                }
                line("Incoming references="+incoming);
                ArrayList<String> outgoing=new ArrayList<>();
                InstructionIterator ins=listing.getInstructions(fn.getBody(),true);
                int limit=0;
                while(ins.hasNext() && limit++<10000){
                    Instruction in=ins.next();
                    if(!in.getFlowType().isCall()) continue;
                    StringBuilder dests=new StringBuilder();
                    for(Address dst:in.getFlows()) dests.append(dst).append(",");
                    outgoing.add(in.getAddress()+" -> "+dests);
                }
                line("Calls="+outgoing);
                if(!seed[2].equals("Y")) {
                    line("Pseudocode omitted: OEM/flash/erase/vendor-sensitive function.");
                    continue;
                }
                if(!opened)continue;
                try{
                    DecompileResults dr=dc.decompileFunction(fn,90,monitor);
                    if(dr==null || !dr.decompileCompleted() ||
                       dr.getDecompiledFunction()==null){
                        line("Decompile failed: "+(dr==null?"no result":dr.getErrorMessage()));
                    } else {
                        String source=dr.getDecompiledFunction().getC();
                        // Export only targeted functions, no OEM password/token literals.
                        // Redact long quoted strings that might embed vendor credentials.
                        source=source.replaceAll(
                            "\"[^\"\\n]{36,}\"",
                            "\"[LONG_LITERAL_REDACTED]\"");
                        if(source.length()>30000) source=source.substring(0,30000)
                            +"\n[TRUNCATED]";
                        line(source);
                    }
                }catch(Exception ex){
                    line("Decompiler error: "+ex);
                }
            }
        } finally {
            dc.dispose();
        }

        line("");
        line("=== REMAINING SAFETY/RECOVERY BOUNDARIES ===");
        line("Bootloader routines are statically identified, not hardware-tested.");
        line("Runtime authorization default and safe DRAM bound remain unproven.");
        line("Immutable RK3288 MaskROM isn't present in this eMMC payload.");
        line("No flash, erase, OEM, reboot or USB device commands executed.");

        String root=System.getenv("MPFREE_REPORT_DIR");
        if(root==null || root.trim().isEmpty()){
            root=System.getProperty("user.home")+"/mpfree-work";
        }
        File dir=new File(root);
        if(!dir.isDirectory() && !dir.mkdirs()){
            throw new IOException("Cannot create report directory "+root);
        }
        String stamp=new SimpleDateFormat("yyyyMMdd'T'HHmmss").format(new Date());
        File report=new File(dir,"mpfree-ghidra-p3-final-"+stamp+".txt");
        try(PrintWriter pw=new PrintWriter(new BufferedWriter(new FileWriter(report)))){
            for(String row:result)pw.println(row);
        }
        info("REPORT: "+report.getAbsolutePath());
        info("final Java pre/postScript analysis finished");
    }
}
