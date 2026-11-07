#!/usr/bin/env python3
"""Reads methods out of a jar as compact pseudo-code, using the JDK's javap.

Game behaviour is checked against the official jars rather than guessed. Raw javap
output is long, so this replays each method's operand stack symbolically and prints
statements with goto-style control flow. It isn't a decompiler: output is for reading
only and is never copied into the project.

    tools/jread.py CLASS                  list fields and methods
    tools/jread.py CLASS NAME [NAME...]   print those methods
    tools/jread.py --raw CLASS NAME       print javap's own text for them
    tools/jread.py --find REGEX           list class names in the jar
    tools/jread.py --strings CLASS        list the string constants in a class

Classes use dots or slashes; a bare name such as LivingEntity is searched for.
Set JAR to read a different jar (default: target/reference/client.jar).
"""

import os
import pathlib
import re
import subprocess
import sys
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
JAR = os.environ.get("JAR", str(ROOT / "target/reference/client.jar"))
JAVAP = str(ROOT / "target/java/bin/javap")


def short(t: str) -> str:
    return re.sub(r"(?:[\w$]+[/.])+([\w$]+)", r"\1", t)


def parse_desc(desc: str) -> tuple[list[str], str]:
    args = []
    i = desc.index("(") + 1

    def one(i: int) -> tuple[str, int]:
        dims = 0
        while desc[i] == "[":
            dims += 1
            i += 1
        c = desc[i]
        if c == "L":
            j = desc.index(";", i)
            name = desc[i + 1 : j].split("/")[-1]
            i = j + 1
        else:
            name = {"I": "int", "J": "long", "F": "float", "D": "double", "Z": "boolean", "B": "byte", "S": "short", "C": "char", "V": "void"}[c]
            i += 1
        return name + "[]" * dims, i

    while desc[i] != ")":
        a, i = one(i)
        args.append(a)
    ret, _ = one(i + 1)
    return args, ret


BIN = {"add": "+", "sub": "-", "mul": "*", "div": "/", "rem": "%", "shl": "<<", "shr": ">>", "ushr": ">>>", "and": "&", "or": "|", "xor": "^"}
CMP = {"eq": "==", "ne": "!=", "lt": "<", "ge": ">=", "gt": ">", "le": "<="}
NEG = {"==": "!=", "!=": "==", "<": ">=", ">=": "<", ">": "<=", "<=": ">"}
CONV = {"f": "float", "d": "double", "i": "int", "l": "long", "b": "byte", "c": "char", "s": "short"}


def strip_parens(e: str) -> str:
    if e.startswith("(") and e.endswith(")"):
        depth = 0
        for i, c in enumerate(e):
            depth += c == "("
            depth -= c == ")"
            if depth == 0 and i != len(e) - 1:
                return e
        return e[1:-1]
    return e


class Stack(list):
    """Catch handlers start with an exception on the stack that the listing doesn't show."""

    def pop(self, *a):
        return super().pop(*a) if self else "$caught"


class Method:
    def __init__(self, header: str, static: bool, desc: str):
        self.header = header
        self.static = static
        self.desc = desc
        self.code: list[tuple[int, str, str, str]] = []
        self.switches: dict[int, list[tuple[str, int]]] = {}

    def render(self) -> str:
        out = []
        stack = Stack()
        at_label: dict[int, list[str]] = {}
        targets = set()
        for off, op, args, _ in self.code:
            if op.startswith("if") or op in ("goto", "goto_w"):
                targets.add(int(args.split()[0]))
            if off in self.switches:
                targets.update(t for _, t in self.switches[off])
        spill = 0

        def local(n: str) -> str:
            return "this" if n == "0" and not self.static else f"v{n}"

        def flush(target: int | None) -> None:
            nonlocal stack, spill
            if stack:
                names = []
                for i, e in enumerate(stack):
                    name = f"$s{i}"
                    if e != name:
                        out.append(f"  {name} = {strip_parens(e)}")
                    names.append(name)
                stack = Stack(names)
            if target is not None:
                at_label[target] = list(stack)

        dead = False
        for off, op, args, comment in self.code:
            if off in targets:
                if not dead:
                    flush(off)
                stack = Stack(at_label.get(off, []))
                out.append(f"L{off}:")
            dead = False
            m = re.match(r"([aifdl])load(?:_(\d))?$", op)
            if m:
                stack.append(local(m.group(2) or args.split()[0]))
                continue
            m = re.match(r"([aifdl])store(?:_(\d))?$", op)
            if m:
                out.append(f"  {local(m.group(2) or args.split()[0])} = {strip_parens(stack.pop())}")
                continue
            m = re.match(r"([ifdl])const_(m1|\d)$", op)
            if m:
                v = "-1" if m.group(2) == "m1" else m.group(2)
                stack.append(v + {"f": "f", "d": ".0", "l": "L", "i": ""}[m.group(1)])
                continue
            if op == "aconst_null":
                stack.append("null")
            elif op in ("bipush", "sipush"):
                stack.append(args)
            elif op in ("ldc", "ldc_w", "ldc2_w"):
                num = re.match(r"(float|double|long|int)\s+(.*)", comment)
                if num:
                    stack.append(num.group(2))
                elif comment.startswith("String "):
                    stack.append(repr(comment[7:]))
                elif comment.startswith("class "):
                    stack.append(short(comment[6:]) + ".class")
                else:
                    stack.append(short(comment))
            elif op == "getfield":
                name = comment.replace("Field ", "").split(":")[0].split(".")[-1]
                stack.append(f"{stack.pop()}.{name}")
            elif op == "getstatic":
                # Keep the owner: "Foo.STREAM_CODEC" says much more than "STREAM_CODEC".
                name = comment.replace("Field ", "").split(":")[0]
                owner, _, field = name.rpartition(".")
                stack.append(f"{owner.rsplit('/', 1)[-1]}.{field}" if owner else field)
            elif op == "putfield":
                v, o = stack.pop(), stack.pop()
                name = comment.replace("Field ", "").split(":")[0].split(".")[-1]
                out.append(f"  {o}.{name} = {strip_parens(v)}")
            elif op == "putstatic":
                out.append(f"  {short(comment.replace('Field ', '').split(':')[0])} = {strip_parens(stack.pop())}")
            elif op.startswith("invoke") and op != "invokedynamic":
                kind = comment.split()[0]
                sig = comment[len(kind) + 1 :].strip()
                name_part, desc = sig.split(":", 1)
                argt, ret = parse_desc(desc)
                call_args = [strip_parens(stack.pop()) for _ in argt][::-1]
                name = name_part.split(".")[-1].strip('"')
                owner = short(name_part.rsplit(".", 1)[0]) if "." in name_part else ""
                if op == "invokestatic":
                    expr = f"{owner + '.' if owner else ''}{name}({', '.join(call_args)})"
                elif name == "<init>":
                    recv = stack.pop()
                    if recv.startswith("NEW:"):
                        if stack and stack[-1] == recv:
                            stack.pop()
                            stack.append(f"new {recv[4:]}({', '.join(call_args)})")
                        else:
                            out.append(f"  new {recv[4:]}({', '.join(call_args)})")
                    else:
                        out.append(f"  super<{owner}>({', '.join(call_args)})")
                    continue
                else:
                    recv = stack.pop()
                    expr = f"{recv}.{name}({', '.join(call_args)})"
                if ret == "void":
                    out.append(f"  {expr}")
                else:
                    stack.append(expr)
            elif op == "invokedynamic":
                m = re.search(r"#\d+:([\w$<>]+):(\(.*)$", comment)
                name, desc = m.group(1), m.group(2)
                argt, _ = parse_desc(desc)
                call_args = [strip_parens(stack.pop()) for _ in argt][::-1]
                stack.append(f"λ{name}[{', '.join(call_args)}]")
            elif op == "new":
                stack.append("NEW:" + short(comment.replace("class ", "")))
            elif op == "dup":
                stack.append(stack[-1])
            elif op == "dup_x1":
                a, b = stack.pop(), stack.pop()
                stack += [a, b, a]
            elif op == "dup_x2":
                a, b, c = stack.pop(), stack.pop(), stack.pop()
                stack += [a, c, b, a]
            elif op == "dup2":
                # Two values of a category-1 type, or one wide value; the listing reads fine either way.
                if len(stack) >= 2 and re.match(r"^(this|v\d+)$", stack[-2]) and "[" not in stack[-1] and not stack[-1].endswith((".0", "L")):
                    stack += [stack[-2], stack[-1]]
                else:
                    stack.append(stack[-1])
            elif op == "pop" or op == "pop2":
                e = stack.pop()
                if "(" in e:
                    out.append(f"  {strip_parens(e)}")
            elif op == "swap":
                a, b = stack.pop(), stack.pop()
                stack += [a, b]
            elif len(op) > 1 and op[0] in "ifdl" and op[1:] in BIN:
                b, a = stack.pop(), stack.pop()
                stack.append(f"({a} {BIN[op[1:]]} {b})")
            elif len(op) == 4 and op[0] in "ifdl" and op[1:] == "neg":
                stack.append(f"-{stack.pop()}")
            elif op in ("fcmpl", "fcmpg", "dcmpl", "dcmpg", "lcmp"):
                b, a = stack.pop(), stack.pop()
                stack.append(f"CMP({a}, {b})")
            elif len(op) == 3 and op[1] == "2" and op[0] in CONV and op[2] in CONV:
                stack.append(f"({CONV[op[2]]}){stack.pop()}")
            elif op == "checkcast":
                stack.append(f"(({short(comment.replace('class ', ''))}){stack.pop()})")
            elif op == "instanceof":
                stack.append(f"({stack.pop()} instanceof {short(comment.replace('class ', ''))})")
            elif op == "arraylength":
                stack.append(f"{stack.pop()}.length")
            elif op == "newarray":
                stack.append(f"new {args}[{stack.pop()}]")
            elif op == "anewarray":
                stack.append(f"new {short(comment.replace('class ', ''))}[{stack.pop()}]")
            elif op == "multianewarray":
                dims = int(args.split()[1].rstrip(","))
                sizes = [stack.pop() for _ in range(dims)][::-1]
                stack.append(f"new {short(comment)}{sizes}")
            elif op.endswith("aload") and len(op) == 6:
                i, a = stack.pop(), stack.pop()
                stack.append(f"{a}[{i}]")
            elif op.endswith("astore") and len(op) == 7:
                v, i, a = stack.pop(), stack.pop(), stack.pop()
                out.append(f"  {a}[{i}] = {strip_parens(v)}")
            elif op == "iinc":
                n, c = args.replace(",", " ").split()
                out.append(f"  {local(n)} += {c}")
            elif op.startswith("if"):
                target = int(args.split()[0])
                if op.startswith("if_icmp") or op.startswith("if_acmp"):
                    b, a = stack.pop(), stack.pop()
                    cond = f"{a} {CMP[op[7:]]} {b}"
                elif op == "ifnull":
                    cond = f"{stack.pop()} == null"
                elif op == "ifnonnull":
                    cond = f"{stack.pop()} != null"
                else:
                    e = stack.pop()
                    c = op[2:]
                    m = re.match(r"^CMP\((.*)\)$", e)
                    if m:
                        depth, split = 0, None
                        s = m.group(1)
                        for i, ch in enumerate(s):
                            depth += ch in "(["
                            depth -= ch in ")]"
                            if ch == "," and depth == 0:
                                split = i
                                break
                        cond = f"{s[:split]} {CMP[c]} {s[split + 2:]}" if split is not None else f"{e} {CMP[c]} 0"
                    elif c == "eq":
                        cond = f"!{e}"
                    elif c == "ne":
                        cond = e
                    else:
                        cond = f"{e} {CMP[c]} 0"
                flush(target)
                out.append(f"  if ({strip_parens(cond)}) goto L{target}")
            elif op in ("goto", "goto_w"):
                target = int(args.split()[0])
                flush(target)
                out.append(f"  goto L{target}")
                dead = True
            elif op in ("tableswitch", "lookupswitch"):
                v = stack.pop()
                flush(None)
                cases = ", ".join(f"{k}: L{t}" for k, t in self.switches.get(off, []))
                for _, t in self.switches.get(off, []):
                    at_label[t] = list(stack)
                out.append(f"  switch ({strip_parens(v)}) {{ {cases} }}")
                dead = True
            elif op in ("ireturn", "lreturn", "freturn", "dreturn", "areturn"):
                out.append(f"  return {strip_parens(stack.pop())}")
                dead = True
            elif op == "return":
                out.append("  return")
                dead = True
            elif op == "athrow":
                out.append(f"  throw {stack.pop()}")
                dead = True
            elif op in ("monitorenter", "monitorexit"):
                stack.pop()
            elif op == "nop":
                pass
            else:
                out.append(f"  ?{op} {args} {comment}")
        return "\n".join(out)


def javap(cls: str) -> str:
    r = subprocess.run([JAVAP, "-c", "-p", "-s", "-constants", "-cp", JAR, cls], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(r.stdout + r.stderr)
    return r.stdout


def parse(text: str) -> tuple[list[str], list[Method]]:
    fields: list[str] = []
    methods: list[Method] = []
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("  ") and not line.startswith("   ") and line.rstrip().endswith(";"):
            header = line.strip()
            desc = ""
            if i + 1 < len(lines) and "descriptor:" in lines[i + 1]:
                desc = lines[i + 1].split("descriptor:")[1].strip()
                i += 1
            if "(" in header or desc.startswith("("):
                m = Method(short(header.rstrip(";")), " static " in f" {header} " or header.startswith("static "), desc)
                methods.append(m)
                i += 1
                if i < len(lines) and lines[i].strip() == "Code:":
                    i += 1
                    while i < len(lines):
                        code = re.match(r"\s+(\d+): (\w+)\s*([^/]*?)(?:\s*// (.*))?$", lines[i])
                        if not code:
                            break
                        off, op, args, comment = int(code.group(1)), code.group(2), code.group(3).strip(), code.group(4) or ""
                        m.code.append((off, op, args, comment))
                        i += 1
                        if op in ("tableswitch", "lookupswitch"):
                            cases = []
                            while i < len(lines) and lines[i].strip() != "}":
                                c = re.match(r"\s*(\w+|-?\d+): (\d+)", lines[i])
                                if c:
                                    cases.append((c.group(1), int(c.group(2))))
                                i += 1
                            m.switches[off] = cases
                            i += 1
                    continue
                continue
            fields.append(short(header.rstrip(";")))
        i += 1
    return fields, methods


def names() -> list[str]:
    with zipfile.ZipFile(JAR) as z:
        return [n[:-6].replace("/", ".") for n in z.namelist() if n.endswith(".class")]


def resolve(cls: str) -> str:
    cls = cls.replace("/", ".")
    if "." in cls:
        return cls
    found = [n for n in names() if n.rsplit(".", 1)[-1] == cls]
    if len(found) == 1:
        return found[0]
    if not found:
        sys.exit(f"no class named {cls}")
    sys.exit(f"{cls} is ambiguous:\n  " + "\n  ".join(found))


def main() -> None:
    args = sys.argv[1:]
    raw = "--raw" in args
    if raw:
        args.remove("--raw")
    if not args:
        sys.exit(__doc__)
    if args[0] == "--find":
        pat = re.compile(args[1])
        print("\n".join(n for n in names() if pat.search(n)))
        return
    if args[0] == "--strings":
        text = subprocess.run([JAVAP, "-v", "-p", "-cp", JAR, resolve(args[1])], capture_output=True, text=True).stdout
        print("\n".join(sorted(set(re.findall(r"= String\s+(.*)$", text, re.M)))))
        return
    cls = resolve(args[0])
    fields, methods = parse(javap(cls))
    wanted = args[1:]
    if not wanted:
        print(cls)
        print("fields:")
        for f in fields:
            print(" ", f)
        print("methods:")
        for m in methods:
            print(" ", m.header)
        return
    for m in methods:
        name = re.search(r"([\w$<>]+)\(", m.header)
        title = name.group(1) if name else ("<clinit>" if "static {}" in m.header else "")
        if title in wanted or (title == cls.rsplit(".", 1)[-1] and "<init>" in wanted):
            print(f"\n{m.header}")
            if raw:
                for off, op, a, c in m.code:
                    print(f"  {off}: {op} {a} {('// ' + c) if c else ''}")
            else:
                print(m.render())


if __name__ == "__main__":
    main()
