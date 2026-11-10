#!/usr/bin/env python3
"""OFFLINE, workstation only: read and compare flattened device trees (.dtb files).

Commands:
  boards DTB...        one line per file: file name, model and compatible
  match COMPAT DTB...  the files whose root compatible list contains COMPAT exactly
  same-board LIVE DTB...
                       the files whose root compatible list is the same as
                       LIVE's, in the same order: the board file U-Boot picked
                       (for the MPC One, rk3288-az01-acva.dtb, not -acva-c)
  summary DTB...       what matters for booting a kernel by kexec: model,
                       compatible, the storage controllers (with their status
                       and aliases), the memory node and the /chosen node
  diff A B [--values]  every node and property that differs between two trees.
                       Values are left out unless --values is given, so it is
                       safe to run on a tree read from the MPC, which carries
                       the unit's serial number and MAC address.
  show DTB NODE...     each NODE (a full path, such as /ethernet@ff290000) with
                       everything under it, values included, then every node
                       those point to by phandle (its own properties only).
                       A phandle is printed as the path of the node it points
                       to. The values of local-mac-address, mac-address,
                       serial-number, bootargs, rng-seed and kaslr-seed are
                       never printed, so it is safe on a tree read from the MPC.
  kexec-dtb --target NEW.dtb --reference OLD.dtb --live LIVE.dtb --out OUT.dtb
            [--drop /path/property]... [--add-compatible /path=STRING]...
                       build the device tree for booting another kernel by
                       kexec: NEW (the other version's board file) plus what
                       U-Boot added at boot (the differences from OLD, the
                       running version's board file, to LIVE, the running
                       tree), with the eMMC and SD card controllers
                       disabled, so the other kernel sees neither. bootargs,
                       initrd and random-seed properties of /chosen are not
                       copied, because kexec sets them, and neither are
                       U-Boot's memory reservations (it reserves the tree's
                       own old location; the new kernel reserves its tree
                       itself). --drop removes a property from the result.
                       --add-compatible puts STRING first in a node's
                       compatible list (for an Ethernet PHY node,
                       "ethernet-phy-idXXXX.XXXX" makes the kernel take the
                       PHY's ID from the tree instead of reading it from a
                       PHY that may still be held in reset).
                       OUT must not exist. The printed report names
                       properties only, never values.

It only reads the files it is given and prints to stdout. It never contacts
the MPC.

The parser and writer here are shared with the tool that builds the device
tree for a kexec boot test, so the round trip is covered by the tests:
parse(build(tree)) gives back the same tree.
"""
import copy
import os
import re
import struct
import sys

FDT_MAGIC = 0xD00DFEED
FDT_BEGIN_NODE, FDT_END_NODE, FDT_PROP, FDT_NOP, FDT_END = 1, 2, 3, 4, 9
HEADER = struct.Struct(">10I")
MAX_DEPTH = 64
MAX_SIZE = 4 << 20
EMMC_ADDRESS = "ff0f0000"  # RK3288 eMMC controller (dw-mshc) on the MPC One
SD_ADDRESS = "ff0c0000"    # RK3288 SD card controller
# Controllers the kernel booted by kexec must not see: then no program it runs
# can reach the eMMC or a card in the SD slot.
HIDDEN_CONTROLLERS = (("eMMC", EMMC_ADDRESS), ("SD card", SD_ADDRESS))


class DtbError(Exception):
    pass


class Node:
    def __init__(self, name):
        self.name = name
        self.props = []      # [name, bytes value], in file order
        self.children = []

    def prop(self, name):
        for pname, value in self.props:
            if pname == name:
                return value
        return None

    def set_prop(self, name, value):
        for entry in self.props:
            if entry[0] == name:
                entry[1] = value
                return
        self.props.append([name, value])

    def del_prop(self, name):
        self.props = [p for p in self.props if p[0] != name]

    def child(self, name):
        for c in self.children:
            if c.name == name:
                return c
        return None


class Tree:
    def __init__(self, root, reserved=(), boot_cpuid=0):
        self.root, self.reserved, self.boot_cpuid = root, list(reserved), boot_cpuid

    def walk(self):
        """(path, node) for every node, parents first."""
        stack = [("/", self.root)]
        while stack:
            path, node = stack.pop(0)
            yield path, node
            for c in node.children:
                stack.append(((path.rstrip("/") + "/" + c.name), c))

    def find(self, path):
        node = self.root
        for part in [p for p in path.split("/") if p]:
            node = node.child(part)
            if node is None:
                return None
        return node


def parse(blob):
    if len(blob) < HEADER.size:
        raise DtbError("file too short for an FDT header")
    (magic, total, off_struct, off_strings, off_rsv, version, _last_comp, boot_cpuid,
     size_strings, size_struct) = HEADER.unpack_from(blob, 0)
    if magic != FDT_MAGIC:
        raise DtbError(f"not a device tree (starts {blob[:4].hex()})")
    if total > len(blob) or total > MAX_SIZE:
        raise DtbError(f"header says {total} bytes, file has {len(blob)}")
    if version < 16:
        raise DtbError(f"FDT version {version} is too old")
    for off, length, what in ((off_struct, size_struct, "structure"), (off_strings, size_strings, "strings")):
        if off + length > total:
            raise DtbError(f"FDT {what} block runs past the end")
    reserved, pos = [], off_rsv
    while True:
        if pos + 16 > total:
            raise DtbError("memory reservation map runs past the end")
        addr, size = struct.unpack_from(">QQ", blob, pos)
        pos += 16
        if addr == 0 and size == 0:
            break
        reserved.append((addr, size))
    strings = blob[off_strings:off_strings + size_strings]
    pos, end, stack, root = off_struct, off_struct + size_struct, [], None
    while pos < end:
        token = struct.unpack_from(">I", blob, pos)[0]
        pos += 4
        if token == FDT_BEGIN_NODE:
            nul = blob.find(b"\x00", pos, end)
            if nul < 0:
                raise DtbError(f"unterminated node name at offset {pos}")
            name = blob[pos:nul].decode("ascii", "replace")
            pos = (nul + 1 + 3) & ~3
            node = Node(name)
            if not stack:
                if root is not None:
                    raise DtbError("more than one root node")
                root = node
            else:
                stack[-1].children.append(node)
            stack.append(node)
            if len(stack) > MAX_DEPTH:
                raise DtbError("nesting is implausibly deep")
        elif token == FDT_END_NODE:
            if not stack:
                raise DtbError(f"unbalanced end of node at offset {pos - 4}")
            stack.pop()
        elif token == FDT_PROP:
            if not stack or pos + 8 > end:
                raise DtbError(f"bad property at offset {pos - 4}")
            length, name_off = struct.unpack_from(">II", blob, pos)
            pos += 8
            name_end = strings.find(b"\x00", name_off)
            if name_off >= len(strings) or name_end < 0:
                raise DtbError(f"property name offset {name_off} is outside the strings block")
            if pos + length > end:
                raise DtbError("property value runs past the structure block")
            stack[-1].props.append([strings[name_off:name_end].decode("ascii", "replace"),
                                    bytes(blob[pos:pos + length])])
            pos = (pos + length + 3) & ~3
        elif token == FDT_NOP:
            continue
        elif token == FDT_END:
            break
        else:
            raise DtbError(f"unknown token 0x{token:x} at offset {pos - 4}")
    if stack:
        raise DtbError("structure block ends inside a node")
    if root is None:
        raise DtbError("no root node")
    return Tree(root, reserved, boot_cpuid)


def build(tree):
    """Serialise a Tree as an FDT version 17 blob."""
    strings, offsets = bytearray(), {}

    def name_offset(name):
        if name not in offsets:
            offsets[name] = len(strings)
            strings.extend(name.encode("ascii") + b"\x00")
        return offsets[name]

    struct_block = bytearray()

    def pad():
        while len(struct_block) % 4:
            struct_block.append(0)

    def emit(node):
        struct_block.extend(struct.pack(">I", FDT_BEGIN_NODE))
        struct_block.extend(node.name.encode("ascii") + b"\x00")
        pad()
        for pname, value in node.props:
            struct_block.extend(struct.pack(">III", FDT_PROP, len(value), name_offset(pname)))
            struct_block.extend(value)
            pad()
        for c in node.children:
            emit(c)
        struct_block.extend(struct.pack(">I", FDT_END_NODE))

    emit(tree.root)
    struct_block.extend(struct.pack(">I", FDT_END))
    rsv = b"".join(struct.pack(">QQ", a, s) for a, s in tree.reserved) + struct.pack(">QQ", 0, 0)
    off_rsv = HEADER.size  # 40, already 8-byte aligned
    off_struct = off_rsv + len(rsv)
    off_strings = off_struct + len(struct_block)
    total = off_strings + len(strings)
    header = HEADER.pack(FDT_MAGIC, total, off_struct, off_strings, off_rsv, 17, 16,
                         tree.boot_cpuid, len(strings), len(struct_block))
    return header + rsv + bytes(struct_block) + bytes(strings)


def load(path):
    with open(path, "rb") as fh:
        blob = fh.read(MAX_SIZE + 1)
    if len(blob) > MAX_SIZE:
        raise DtbError(f"{path} is larger than {MAX_SIZE} bytes")
    return parse(blob)


def strings_of(value):
    """The NUL-separated strings in value, or None if it isn't a printable string list."""
    if not value or value[-1:] != b"\x00":
        return None
    parts = value[:-1].split(b"\x00")
    if any(not p or any(b < 0x20 or b > 0x7e for b in p) for p in parts):
        return None
    return [p.decode("ascii") for p in parts]


def show(value, max_cells=32, max_bytes=64):
    texts = strings_of(value)
    if texts is not None:
        return ", ".join(f'"{t}"' for t in texts)
    if value and len(value) % 4 == 0:
        cells = struct.unpack(f">{len(value) // 4}I", value)
        shown = " ".join(f"0x{c:x}" for c in cells[:max_cells])
        return f"<{shown}{' ...' if len(cells) > max_cells else ''}>"
    if not value:
        return "(empty)"
    return f"[{value[:max_bytes].hex()}{'...' if len(value) > max_bytes else ''}]"


# Properties whose values `show` never prints: they identify the unit (U-Boot
# adds them at boot) or hold boot-time secrets.
NOT_SHOWN = ("local-mac-address", "mac-address", "serial-number", "bootargs", "rng-seed", "kaslr-seed")
# Phandle lists: property -> the property of the node pointed to that says how
# many argument cells follow each phandle.
ARG_CELLS = {
    "clocks": "#clock-cells", "assigned-clocks": "#clock-cells", "assigned-clock-parents": "#clock-cells",
    "resets": "#reset-cells", "phys": "#phy-cells", "power-domains": "#power-domain-cells",
    "dmas": "#dma-cells", "pwms": "#pwm-cells", "io-channels": "#io-channel-cells",
    "interrupts-extended": "#interrupt-cells", "iommus": "#iommu-cells", "mboxes": "#mbox-cells",
    "sound-dai": "#sound-dai-cells",
}
# Properties in which every cell is a phandle.
ALL_PHANDLES = re.compile(r"^(.*-supply|.*-handle|pinctrl-[0-9]+|interrupt-parent|memory-region|phy|"
                          r"remote-endpoint|rockchip,grf|rockchip,pmu|backlight|panel|operating-points-v2)$")


def phandles(tree):
    """{phandle: path} from every node's phandle (or linux,phandle) property."""
    found = {}
    for path, node in tree.walk():
        for name in ("phandle", "linux,phandle"):
            value = node.prop(name)
            if value is not None and len(value) == 4:
                found[struct.unpack(">I", value)[0]] = path
    return found


def references(name, value, handles, nodes):
    """[(path, argument cells)] for a property that holds phandles, or None if it doesn't parse as one."""
    if not value or len(value) % 4:
        return None
    cells = struct.unpack(f">{len(value) // 4}I", value)
    if name == "rockchip,pins":  # <bank pin function &pin-config> per pin
        if len(cells) % 4 or any(cells[i] not in handles for i in range(3, len(cells), 4)):
            return None
        return [(handles[cells[i + 3]], cells[i:i + 3]) for i in range(0, len(cells), 4)]
    if ALL_PHANDLES.match(name):
        if any(c not in handles for c in cells):
            return None
        return [(handles[c], ()) for c in cells]
    count = ARG_CELLS.get(name)
    if count is None and (name in ("gpio", "gpios") or name.endswith(("-gpio", "-gpios"))):
        count = "#gpio-cells"
    if count is None:
        return None
    refs, i = [], 0
    while i < len(cells):
        path = handles.get(cells[i])
        n = nodes[path].prop(count) if path is not None else None
        if n is None or len(n) != 4:
            return None
        n = struct.unpack(">I", n)[0]
        if i + 1 + n > len(cells):
            return None
        refs.append((path, cells[i + 1:i + 1 + n]))
        i += 1 + n
    return refs


def show_value(name, value, handles, nodes):
    if name in NOT_SHOWN:
        return f"({len(value)} bytes, not shown)", []
    refs = references(name, value, handles, nodes)
    if refs is None:
        return show(value, max_cells=1024, max_bytes=1024), []
    if name == "rockchip,pins":
        text = ", ".join(f"<{' '.join(f'0x{c:x}' for c in args)} &{{{path}}}>" for path, args in refs)
    else:
        text = ", ".join(f"<&{{{path}}}{''.join(f' 0x{c:x}' for c in args)}>" for path, args in refs)
    return text, [path for path, _ in refs]


def show_nodes(path, wanted, out):
    """Print each wanted node with its subtree, then the nodes they point to. Returns how many were missing."""
    tree = load(path)
    nodes, handles = dict(tree.walk()), phandles(tree)
    out.write(f"--- {path}\n")
    printed, pointed, missing = set(), [], 0

    def emit(node_path, node, depth, children):
        pad = "  " * depth
        out.write(f"{pad}{node_path if depth == 0 else node.name} {{\n")
        printed.add(node_path)
        for name, value in node.props:
            text, targets = show_value(name, value, handles, nodes)
            out.write(f"{pad}  {name} = {text};\n")
            pointed.extend(t for t in targets if t not in pointed)
        if children:
            for c in node.children:
                emit(node_path.rstrip("/") + "/" + c.name, c, depth + 1, True)
        out.write(f"{pad}}};\n")

    for want in wanted:
        node = tree.find(want) if want.startswith("/") else None
        if node is None:
            missing += 1
            out.write(f"(no node {want} in this tree)\n")
            continue
        emit(want if want == "/" else "/" + "/".join(p for p in want.split("/") if p), node, 0, True)
    extra = [p for p in pointed if p not in printed]
    if extra:
        out.write("nodes pointed to from the nodes above (their own properties only):\n")
        for p in extra:
            emit(p, nodes[p], 0, False)
    return missing


def unit_address(name):
    return name.split("@", 1)[1] if "@" in name else ""


def nodes_at(tree, address):
    """(path, node) of every node whose unit address is this one."""
    return [(p, n) for p, n in tree.walk() if unit_address(n.name).lower() == address]


def status_of(node):
    texts = strings_of(node.prop("status") or b"")
    return texts[0] if texts else "okay (no status property)"


def aliases_to(tree, path):
    aliases = tree.find("/aliases")
    if aliases is None:
        return []
    return [name for name, value in aliases.props if strings_of(value) == [path]]


def compatible(tree):
    return strings_of(tree.root.prop("compatible") or b"") or []


def model(tree):
    texts = strings_of(tree.root.prop("model") or b"")
    return texts[0] if texts else "(no model)"


def storage_nodes(tree):
    """Every MMC/SD/eMMC controller: dw-mshc or sdhci compatibles, plus anything at the eMMC/SD addresses."""
    found = []
    for path, node in tree.walk():
        compat = " ".join(strings_of(node.prop("compatible") or b"") or [])
        if ("dw-mshc" in compat or "sdhci" in compat or
                unit_address(node.name).lower() in (EMMC_ADDRESS, SD_ADDRESS)):
            found.append((path, node))
    return found


def summary(path, out):
    tree = load(path)
    out.write(f"--- {path}\n")
    out.write(f"model: {model(tree)}\n")
    out.write(f"compatible: {', '.join(compatible(tree))}\n")
    out.write(f"memory reservations: {len(tree.reserved)}\n")
    for p, node in storage_nodes(tree):
        compat = ", ".join(strings_of(node.prop("compatible") or b"") or ["(none)"])
        aliases = ", ".join(aliases_to(tree, p)) or "none"
        out.write(f"storage controller {p}: {compat}; status {status_of(node)}; aliases: {aliases}\n")
    emmc = nodes_at(tree, EMMC_ADDRESS)
    out.write(f"nodes at the eMMC address {EMMC_ADDRESS}: {len(emmc)}\n")
    for p, node in tree.walk():
        if node.name == "memory" or node.name.startswith("memory@"):
            reg = node.prop("reg")
            out.write(f"memory node {p}: reg {show(reg) if reg is not None else '(none)'}\n")
    chosen = tree.find("/chosen")
    if chosen is None:
        out.write("/chosen: absent\n")
    else:
        out.write("/chosen properties: " + (", ".join(f"{n} ({len(v)} bytes)" for n, v in chosen.props)
                                            or "none") + "\n")
    out.write("root properties: " + ", ".join(f"{n} ({len(v)} bytes)" for n, v in tree.root.props) + "\n")


def boards(paths, out):
    for path in paths:
        try:
            tree = load(path)
        except (OSError, DtbError) as err:
            out.write(f"{path}: ERROR {err}\n")
            continue
        out.write(f"{path}: {model(tree)}; {', '.join(compatible(tree))}\n")


def diff(a_path, b_path, values, out):
    a, b = load(a_path), load(b_path)
    a_nodes, b_nodes = dict(a.walk()), dict(b.walk())
    changes = 0
    out.write(f"--- differences from {a_path} (A) to {b_path} (B)\n")
    if a.reserved != b.reserved:
        changes += 1
        out.write(f"memory reservations: A has {len(a.reserved)}, B has {len(b.reserved)}\n")
    for path in sorted(set(a_nodes) | set(b_nodes)):
        if path not in b_nodes:
            changes += 1
            out.write(f"node only in A: {path}\n")
            continue
        if path not in a_nodes:
            changes += 1
            out.write(f"node only in B: {path}\n")
            continue
        a_props, b_props = dict(map(tuple, a_nodes[path].props)), dict(map(tuple, b_nodes[path].props))
        for name in sorted(set(a_props) | set(b_props)):
            va, vb = a_props.get(name), b_props.get(name)
            if va == vb:
                continue
            changes += 1
            where = f"{path.rstrip('/')}/{name}" if path != "/" else f"/{name}"
            if vb is None:
                what = f"only in A ({len(va)} bytes)"
            elif va is None:
                what = f"only in B ({len(vb)} bytes)"
            else:
                what = f"differs (A {len(va)} bytes, B {len(vb)} bytes)"
            if values:
                what += f": A {show(va) if va is not None else '-'} | B {show(vb) if vb is not None else '-'}"
            out.write(f"property {where}: {what}\n")
    out.write(f"{changes} difference(s)\n")
    return changes


CHOSEN_NOT_COPIED = ("bootargs", "linux,initrd-start", "linux,initrd-end", "kaslr-seed", "rng-seed")


def parent_of(path):
    return path.rsplit("/", 1)[0] or "/"


def prop_path(path, name):
    return f"{path.rstrip('/')}/{name}"


def kexec_dtb(target, reference, live, drops=(), adds=()):
    """Apply U-Boot's runtime changes (reference -> live) to target and disable the eMMC.

    Returns (notes, problems). Notes name every change; problems are reasons to stop.
    """
    notes, problems = [], []
    ref_nodes = dict(reference.walk())
    live_nodes = dict(live.walk())
    for path, lnode in live.walk():
        rnode = ref_nodes.get(path)
        if rnode is None:
            if parent_of(path) not in ref_nodes:
                continue  # inside a node that is copied whole
            tparent = target.find(parent_of(path))
            if tparent is None:
                problems.append(f"U-Boot added node {path}, but the new tree has no {parent_of(path)}")
            elif target.find(path) is not None:
                problems.append(f"U-Boot added node {path}, but the new tree already has one")
            else:
                tparent.children.append(copy.deepcopy(lnode))
                notes.append(f"copied node added by U-Boot: {path}")
            continue
        for name, value in lnode.props:
            if rnode.prop(name) == value:
                continue
            if path == "/chosen" and name in CHOSEN_NOT_COPIED:
                notes.append(f"not copied (kexec sets it): {prop_path(path, name)}")
                continue
            tnode = target.find(path)
            if tnode is None:
                problems.append(f"U-Boot set {prop_path(path, name)}, but the new tree has no {path}")
                continue
            what = "replaced" if tnode.prop(name) is not None else "added"
            tnode.set_prop(name, value)
            notes.append(f"{what} from the running tree ({len(value)} bytes): {prop_path(path, name)}")
        for name, _ in rnode.props:
            if lnode.prop(name) is None:
                problems.append(f"U-Boot removed {prop_path(path, name)}; decide by hand")
    for path in ref_nodes:
        if path not in live_nodes and parent_of(path) in live_nodes:
            problems.append(f"U-Boot removed node {path}; decide by hand")
    for item in drops:
        path, name = parent_of(item), item.rsplit("/", 1)[1]
        node = target.find(path)
        if node is None or node.prop(name) is None:
            problems.append(f"--drop {item}: no such property in the result")
            continue
        node.del_prop(name)
        notes.append(f"dropped as asked: {item}")
    for item in adds:
        path, _, text = item.partition("=")
        node = target.find(path) if path.startswith("/") else None
        old = strings_of(node.prop("compatible") or b"") if node is not None else None
        if node is None or old is None or not text or strings_of(text.encode() + b"\x00") != [text]:
            problems.append(f"--add-compatible {item}: no such node with a compatible list, or a bad string")
            continue
        if text in old:
            problems.append(f"--add-compatible {item}: the node already has it")
            continue
        node.set_prop("compatible", b"".join(t.encode() + b"\x00" for t in [text] + old))
        notes.append(f"compatible of {path} is now {show(node.prop('compatible'))}")
    if live.reserved != target.reserved:
        notes.append(f"not copied: memory reservations (running tree {len(live.reserved)}, "
                     f"result keeps the new file's {len(target.reserved)})")
    for label, address in HIDDEN_CONTROLLERS:
        found = nodes_at(target, address)
        if len(found) != 1:
            problems.append(f"expected one node at the {label} address {address} in the new tree, found {len(found)}")
            continue
        path, node = found[0]
        node.set_prop("status", b"disabled\x00")
        notes.append(f"{label} controller disabled: {path} status = \"disabled\"")
    return notes, problems


def verify_kexec_dtb(tree, board):
    """Reasons the built tree must not be used (empty if it is fine)."""
    reasons = []
    if board not in compatible(tree):
        reasons.append(f"the tree is not for {board}: {', '.join(compatible(tree))}")
    for label, address in HIDDEN_CONTROLLERS:
        found = nodes_at(tree, address)
        if len(found) != 1 or strings_of(found[0][1].prop("status") or b"") != ["disabled"]:
            reasons.append(f"the {label} controller is not disabled")
    for path, node in tree.walk():
        if (node.name == "memory" or node.name.startswith("memory@")) and \
                not any(node.prop("reg") or b"\x00"):
            reasons.append(f"{path} has no memory size (U-Boot normally fills it in)")
    return reasons


def kexec_dtb_main(args, out):
    opts, drops, adds, i = {}, [], [], 0
    while i < len(args):
        key = args[i]
        if key not in ("--target", "--reference", "--live", "--out", "--drop", "--add-compatible") or \
                i + 1 >= len(args):
            return 2
        if key == "--drop":
            drops.append(args[i + 1])
        elif key == "--add-compatible":
            adds.append(args[i + 1])
        else:
            opts[key] = args[i + 1]
        i += 2
    if set(opts) != {"--target", "--reference", "--live", "--out"}:
        return 2
    if os.path.exists(opts["--out"]):
        raise DtbError(f"{opts['--out']} already exists; not overwriting it")
    target, reference, live = load(opts["--target"]), load(opts["--reference"]), load(opts["--live"])
    board = (compatible(live) or ["?"])[0]
    for label, tree in (("--target", target), ("--reference", reference)):
        if board not in compatible(tree):
            raise DtbError(f"{label} is not a {board} tree: {', '.join(compatible(tree))}")
    notes, problems = kexec_dtb(target, reference, live, drops, adds)
    for note in notes:
        out.write(note + "\n")
    blob = build(target)
    reasons = problems + verify_kexec_dtb(parse(blob), board)
    if reasons:
        for reason in reasons:
            out.write(f"STOP: {reason}\n")
        out.write("No file written.\n")
        return 1
    with open(opts["--out"], "xb") as fh:
        fh.write(blob)
    out.write(f"written: {opts['--out']} ({len(blob)} bytes); its eMMC and SD card controllers are disabled\n")
    return 0


def main(argv):
    args = argv[1:]
    try:
        if len(args) >= 2 and args[0] == "boards":
            boards(args[1:], sys.stdout)
            return 0
        if len(args) >= 3 and args[0] == "match":
            for path in args[2:]:
                if args[1] in compatible(load(path)):
                    print(path)
            return 0
        if len(args) >= 3 and args[0] == "same-board":
            want = compatible(load(args[1]))
            for path in args[2:]:
                if compatible(load(path)) == want:
                    print(path)
            return 0
        if len(args) >= 2 and args[0] == "summary":
            for path in args[1:]:
                summary(path, sys.stdout)
            return 0
        if len(args) >= 3 and args[0] == "show":
            return 1 if show_nodes(args[1], args[2:], sys.stdout) else 0
        if args[:1] == ["kexec-dtb"]:
            rc = kexec_dtb_main(args[1:], sys.stdout)
            if rc != 2:
                return rc
        elif args[:1] == ["diff"] and len(args) in (3, 4) and (len(args) == 3 or args[3] == "--values"):
            diff(args[1], args[2], len(args) == 4, sys.stdout)
            return 0
    except (OSError, DtbError) as err:
        print(f"ERROR: {err}", file=sys.stderr)
        return 1
    print(__doc__.strip(), file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
