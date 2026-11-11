# MPFreeFinishBootChain.py
# @category MPFree
# @menupath Tools.MPFree.Finish p3 U-Boot Analysis
#
# Ghidra Script Manager (Jython 2.7). Operates on a *copy* of the p3 program.
# Re-bases an original base-0 p3 import to its linked base 0x00100000,
# validates the exact 363649-byte image SHA256 and command-table anchors,
# seeds Thumb code, forces fresh analysis, then exports decompiler results.
#
# Does NOT access the MPC, USB, SSH, physical RAM, or eMMC. It DOES modify
# the currently opened Ghidra program database. Duplicate the program
# in the Ghidra project before running if preserving the old state matters.
#
from __future__ import print_function
import os
import time
import re
import traceback
from java.math import BigInteger
from java.security import MessageDigest
from jarray import zeros
from ghidra.program.model.address import AddressSet
from ghidra.program.model.lang import RegisterValue
from ghidra.app.cmd.disassemble import DisassembleCommand
from ghidra.app.decompiler import DecompInterface

BASE = 0x00100000
SIZE = 363649
SHA256 = 'b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
OUTDIR = os.path.expanduser('~/mpfree-work')
# (display label, linked instruction address, emit full pseudocode?)
# OEM pseudocode is deliberately omitted from the shareable report because
# the firmware includes an embedded vendor token. Inspect it locally in Ghidra.
SEEDS = [
    ('bootm_command',         0x00101744, True),
    ('bootm_image_loading',   0x0010A6A4, True),
    ('classify_image',        0x0010E390, True),
    ('legacy_header_crc',     0x0010E000, True),
    ('legacy_payload_crc',    0x0010E02A, True),
    ('legacy_print_metadata', 0x0010E1CC, False),
    ('fdt_header_checker',    0x00127A1C, True),
    ('run_command_candidate', 0x00110484, True),
    ('reset_cpu_candidate',   0x0011622A, False),
    ('usb_exit_state_read',   0x00121DC4, True),
    ('usb_exit_state_set',    0x00121DD0, True),
    ('usb_exit_state_clear',  0x00121DDC, True),
    ('fastboot_continue_done',0x001226B0, True),
    ('fastboot_boot_done',    0x001226B4, True),
    ('fastboot_response',     0x00122734, True),
    ('fastboot_prefix_cmp',   0x00122710, True),
    ('fastboot_dispatch',     0x00122780, True),
    ('fastboot_permission',   0x00122834, True),
    ('fastboot_oem',          0x00122854, False),
    ('fastboot_continue',     0x00122948, True),
    ('fastboot_boot',         0x00122964, True),
    ('fastboot_download',     0x00122980, True),
    ('fastboot_receive',      0x00122A10, True),
    ('fastboot_erase',        0x00122BF8, False),
    ('fastboot_flash',        0x00122C5C, False),
    ('fastboot_reboot',       0x00122CF8, False),
    ('fastboot_getvar',       0x00122D38, True),
    ('memcpy_candidate',      0x0012DDA2, False),
    ('crc_header_candidate',  0x0012DA74, False),
    ('crc_data_candidate',    0x0012DA80, False),
    ('image_loader_candidate',0x0010D14C, False),
    ('fastboot_continue_loop',0x00121DD0, False),
]

GLOBALS = [
    ('fastboot_permission_state', 0x0014D690),
    ('fastboot_received_bytes',   0x0014D680),
    ('fastboot_expected_bytes',   0x0014D684),
    ('fastboot_exit_flag',        0x0014D658),
    ('download_base_value',      0x00143E34),
    ('command_table',            0x0012F2B0),
]

# The exact eight-entry command table is a firm input check, not a discovery.
TABLE = [
    ('reboot',    0x00122CF9),
    ('getvar:',   0x00122D39),
    ('download:', 0x00122981),
    ('boot',      0x00122965),
    ('continue',  0x00122949),
    ('flash',     0x00122C5D),
    ('erase',     0x00122BF9),
    ('oem',       0x00122855),
]

def main():
    try:
        script_args = list(getScriptArgs())
    except Exception:
        script_args = []
    phase = script_args[0] if script_args else 'gui'
    if phase not in ('gui', 'pre', 'post'):
        raise RuntimeError('Unknown script phase: %s' % phase)
    p = currentProgram
    if p is None:
        raise RuntimeError('Open the p3-uimage-payload.bin program first.')
    if str(p.getLanguageID()) != 'ARM:LE:32:v7':
        raise RuntimeError('Expected ARM:LE:32:v7; got %s' % p.getLanguageID())
    mem = p.getMemory()
    candidates = [b for b in mem.getBlocks()
                  if b.isInitialized() and int(b.getSize()) == SIZE]
    if len(candidates) != 1:
        raise RuntimeError('Expected exactly one initialized 363649-byte p3 payload block, found %d. Do not use the uImage with its 64-byte header.' % len(candidates))
    block = candidates[0]
    oldstart = int(block.getStart().getOffset())
    imagebase = int(p.getImageBase().getOffset())
    if oldstart not in (0, BASE):
        raise RuntimeError('Unexpected p3 block base 0x%08X. Nothing modified.' % oldstart)
    if oldstart == 0 and imagebase != 0:
        raise RuntimeError('Block is at zero but Ghidra image base is 0x%08X; refusing an unsafe relative move.' % imagebase)

    space = p.getAddressFactory().getDefaultAddressSpace()
    def a(va):
        return space.getAddress(va)

    def b_at(va):
        return int(mem.getByte(a(va))) & 0xff

    def u32(va):
        return sum(b_at(va+i) << (8*i) for i in range(4))

    # Hash the actual Ghidra bytes using a Java byte[] -- NOT Python bytearray.
    digest = MessageDigest.getInstance('SHA-256')
    done = 0
    while done < SIZE:
        count = min(32768, SIZE - done)
        buf = zeros(count, 'b')
        got = mem.getBytes(a(oldstart + done), buf, 0, count)
        if got != count:
            raise RuntimeError('Could not read %d bytes at +0x%X' % (count, done))
        digest.update(buf)
        done += got
    observed = ''.join('%02x' % (int(x) & 255) for x in digest.digest())
    if observed != SHA256:
        raise RuntimeError('SHA256 mismatch: %s. Nothing modified.' % observed)

    # Confirm exact linked pointer pairs even while current block base is 0.
    table_start = oldstart + 0x2F2B0
    if u32(table_start) != 0x0013F08B or u32(table_start+4) != 0x00122CF9:
        raise RuntimeError('Fastboot table anchor mismatch. Nothing modified.')
    if u32(table_start+0x18+4) != 0x00122965:
        raise RuntimeError('Fastboot boot callback pointer mismatch. Nothing modified.')
    if u32(table_start+0x40) != 0:
        raise RuntimeError('Unexpected table terminator; refusing to modify program.')

    # The actual permanent base fix, with a transaction. This changes Ghidra
    # code/database addresses, NEVER bytes on disk or an address on the MPC.
    if oldstart == 0:
        tx = p.startTransaction('MPFree: rebase p3 to linked 0x00100000')
        ok = False
        try:
            p.setImageBase(a(BASE), True)
            moved = mem.getBlock(a(BASE))
            if moved is None or int(moved.getSize()) != SIZE:
                raise RuntimeError('Ghidra base operation did not relocate the p3 block.')
            ok = True
        finally:
            p.endTransaction(tx, ok)
        println('MPFree: REBASED p3 Ghidra program 00000000 -> 00100000')
    else:
        println('MPFree: p3 memory already begins at 00100000; no rebase needed')

    # Verify after rebase; this also prevents accidentally reading base-0.
    if u32(BASE + 0x2F2B0) != 0x0013F08B:
        raise RuntimeError('Post-rebase anchor check failed.')
    if mem.getBlock(a(BASE)) is None:
        raise RuntimeError('Post-rebase memory block not found.')

    if not os.path.isdir(OUTDIR):
        os.makedirs(OUTDIR)
    report = os.path.join(OUTDIR, 'mpfree-ghidra-p3-final-%s.txt' %
                          time.strftime('%Y%m%dT%H%M%S'))
    notes = []
    def log(s=''):
        notes.append(str(s))

    log('MPFree p3 Ghidra final-pass static analysis')
    log('Program: %s' % p.getName())
    log('Language: %s' % p.getLanguageID())
    log('Validated SHA256: %s' % observed)
    log('Program image base: %s; payload start: %s' % (p.getImageBase(), mem.getBlock(a(BASE)).getStart()))
    log('Code addresses are linked ARMv7 virtual addresses, not live kernel addresses.')
    log('Static analysis cannot establish runtime authorization state, safe DRAM, or prove MaskROM entry.')
    log('PRIVATE NOTE: OEM/vendor token contents intentionally omitted.')
    log()

    tmode = p.getProgramContext().getRegister('TMode')
    if tmode is None:
        raise RuntimeError('Ghidra ARM language has no TMode context register.')
    thumb = RegisterValue(tmode, BigInteger.ONE)

    log('=== SEED ARM THUMB ENTRY POINTS ===')
    for name, va, include_c in SEEDS:
        entry = a(va)
        if not mem.contains(entry):
            log('%-29s 0x%08X outside loaded payload' % (name, va))
            continue
        try:
            existing = p.getListing().getInstructionAt(entry)
            if existing is None:
                cmd = DisassembleCommand(AddressSet(entry, entry), None, True)
                cmd.setInitialContext(thumb)
                worked = cmd.applyTo(p, monitor)
            else:
                worked = True
            instruction = p.getListing().getInstructionAt(entry)
            if instruction is None:
                log('%-29s 0x%08X DISASSEMBLY FAILED (%s)' % (name, va, worked))
                continue
            got_tmode = p.getProgramContext().getValue(tmode, entry, False)
            if got_tmode is None or int(got_tmode.intValue()) != 1:
                log('%-29s 0x%08X WARNING: decoded, but Thumb context not confirmed: %s' %
                    (name, va, got_tmode))
            f = p.getFunctionManager().getFunctionAt(entry)
            if f is None:
                enclosing = p.getFunctionManager().getFunctionContaining(entry)
                if enclosing is None:
                    # Names are conservative and prefixed MPFree rather than
                    # claiming exact original source symbols.
                    f = createFunction(entry, 'mpf_' + name)
                else:
                    log('%-29s 0x%08X inside existing %s at %s' %
                        (name, va, enclosing.getName(), enclosing.getEntryPoint()))
            log('%-29s 0x%08X %s' %
                (name, va, 'function' if f is not None else 'instructions'))
        except Exception as ex:
            log('%-29s 0x%08X ERROR: %s' % (name, va, ex))
    log()

    if phase == 'pre':
        println('MPFree: Thumb seeds prepared before Ghidra automatic analysis')
        return
    if phase == 'gui':
        println('MPFree: Thumb seeds processed, running Auto Analysis')
        try:
            analyzeAll(p)
            log('Auto Analysis: completed')
        except Exception as ex:
            log('Auto Analysis: ERROR %s' % ex)
            log(traceback.format_exc())
    else:
        log('Auto Analysis: executed by Ghidra between pre and post scripts')
    log()

    log('=== COMMAND DISPATCH TABLE (firmware bytes) ===')
    for i, (expected_name, expected_ptr) in enumerate(TABLE):
        row = BASE + 0x2F2B0 + 8*i
        strptr = u32(row)
        fptr = u32(row+4)
        label = ''
        cur = strptr
        if BASE <= cur < BASE + SIZE:
            for j in range(32):
                ch = b_at(cur+j)
                if ch == 0:
                    break
                if 32 <= ch <= 126:
                    label += chr(ch)
                else:
                    label += '?'
        # Only eight known public protocol command strings are emitted.
        if label != expected_name or fptr != expected_ptr:
            log('WARNING unexpected table row %d' % i)
        log('%d %-10s entry=%08X function=%08X thumb=%s matches=%s' %
            (i, expected_name, row, fptr & ~1, bool(fptr & 1),
             label == expected_name and fptr == expected_ptr))
    log()

    rm = p.getReferenceManager()
    fm = p.getFunctionManager()
    listing = p.getListing()
    log('=== GLOBAL STATE REFERENCES (on-disk words are NOT runtime initialization) ===')
    for name, va in GLOBALS:
        raw = u32(va) if mem.contains(a(va+3)) else 0
        refs = []
        it = rm.getReferencesTo(a(va))
        while it.hasNext():
            r = it.next()
            refs.append('%s:%s' % (r.getFromAddress(), r.getReferenceType()))
        log('%-27s %08X fileword=%08X xrefs=%s' %
            (name, va, raw, ', '.join(refs[:80]) or '[none]'))
    log()

    dec = DecompInterface()
    dec.toggleCCode(True)
    dec.toggleSyntaxTree(True)
    dec.setSimplificationStyle('decompile')
    if not dec.openProgram(p):
        log('WARNING: Ghidra Decompiler did not open program.')
    else:
        log('=== FUNCTION-BY-FUNCTION DECOMPILATION AND CALL EDGES ===')
        for name, va, include_c in SEEDS:
            f = fm.getFunctionAt(a(va))
            if f is None:
                enclosing = fm.getFunctionContaining(a(va))
                log('--- %s @ %08X: missing entry function; containing=%s ---' %
                    (name, va, enclosing))
                continue
            log('--- %s @ %08X Ghidra=%s size=%s ---' %
                (name, va, f.getName(), f.getBody().getNumAddresses()))
            inrefs = []
            it = rm.getReferencesTo(a(va))
            while it.hasNext():
                r = it.next()
                inrefs.append(str(r.getFromAddress()) + '/' + str(r.getReferenceType()))
            log('Incoming references: %s' % (', '.join(inrefs[:100]) or '[none]'))
            calls = []
            ins = listing.getInstructions(f.getBody(), True)
            count = 0
            while ins.hasNext() and count < 10000:
                inst = ins.next()
                count += 1
                if inst.getFlowType().isCall():
                    flows = inst.getFlows()
                    dst = ','.join(str(d) for d in flows)
                    calls.append('%s -> %s' % (inst.getAddress(), dst or '[indirect]'))
            log('Direct/indirect calls (%d): %s' %
                (len(calls), ', '.join(calls[:160]) or '[none]'))
            if not include_c:
                log('Pseudocode omitted: sensitive OEM/storage operation or helper; inspect locally in Ghidra.')
                continue
            try:
                result = dec.decompileFunction(f, 90, monitor)
                if result is None or not result.decompileCompleted():
                    log('Decompiler failed: %s' %
                        ('null result' if result is None else result.getErrorMessage()))
                elif result.getDecompiledFunction() is not None:
                    ctext = result.getDecompiledFunction().getC()
                    # Defense in depth if token constants surface in a caller.
                    ctext = re.sub(r'(?i)inmusic-unlock-magic-[A-Za-z0-9_-]+',
                                   '[VENDOR_TOKEN_REDACTED]', ctext)
                    log('C pseudocode follows:')
                    log(ctext[:26000])
                    if len(ctext) > 26000:
                        log('[TRUNCATED: view complete function inside Ghidra]')
            except Exception as ex:
                log('Decompiler exception: %s' % ex)
            log()
    dec.dispose()

    log('=== VERIFY IMPORTANT DIRECT CALLSITE LINKS ===')
    for source, dest, why in [
        (0x001226DE, 0x00101744, 'Fastboot post-ACK -> bootm'),
        (0x0010A758, 0x0010E390, 'bootm -> image classifier'),
        (0x0010A7A6, 0x0010E000, 'bootm -> header CRC'),
        (0x0010A7D0, 0x0010E02A, 'bootm -> data CRC'),
        (0x0012298C, 0x00122834, 'download -> permission check'),
        (0x0012286A, 0x00122834, 'OEM branch -> permission check'),
        (0x00122C00, 0x00122834, 'erase -> permission check'),
        (0x00122C64, 0x00122834, 'flash -> permission check'),
        (0x0010E39A, 0x00127A1C, 'image classifier -> FDT checker'),
    ]:
        inst = listing.getInstructionAt(a(source))
        flows = [] if inst is None else [int(x.getOffset()) for x in inst.getFlows()]
        log('%-39s %08X -> %08X Ghidra=%s' %
            (why, source, dest, 'CONFIRMED' if dest in flows else
             ('NOT YET DECODED' if inst is None else str(flows))))
    log()
    log('=== LIMITATIONS ===')
    log('Full p3 user-program control flow is not proof of a functioning boot on hardware.')
    log('The 1.75-GiB Fastboot protocol limit is NOT a safe DRAM allocation limit.')
    log('Runtime state of 0x0014D690, MCU/mux reset state, BootROM USB entry and DRAM carveouts still require separate evidence.')
    log('No physical MPC/eMMC/USB device was contacted or changed by this Ghidra script.')
    with open(report, 'w') as handle:
        handle.write('\n'.join(notes) + '\n')
    println('MPFree Ghidra REPORT: %s' % report)
    println('MPFree: rebase, targeted Thumb disassembly, analysis and decompiler export complete.')

main()
