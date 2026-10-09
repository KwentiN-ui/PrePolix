"""Erzeugt die Beispiel-Eingabedateien in diesem Ordner.

Aufruf: python3 testdata/erzeugen.py
Die Modelle mit Step lassen sich direkt mit CalculiX rechnen (ccx -i <name>).
"""

import itertools
import os

HERE = os.path.dirname(os.path.abspath(__file__))

STEEL_STEP = """*MATERIAL, NAME=STEEL
*ELASTIC
210000, 0.3
*SOLID SECTION, ELSET=EALL, MATERIAL=STEEL
*STEP
*STATIC
*BOUNDARY
FIX, 1, 3
*CLOAD
{load}
*NODE FILE
U
*EL FILE
S
*END STEP
"""


def write(name, text):
    with open(os.path.join(HERE, name), "w", newline="\n") as f:
        f.write(text)


def grid(nx, ny, nz, size, order):
    """Structured grid; order 2 adds midside points. Returns id lookup and node lines."""
    steps = [n * order for n in (nx, ny, nz)]
    ids, lines = {}, []
    for k, j, i in itertools.product(range(steps[2] + 1), range(steps[1] + 1), range(steps[0] + 1)):
        odd = (i % 2) + (j % 2) + (k % 2) if order == 2 else 0
        if odd > 1:
            continue
        nid = len(ids) + 1
        ids[(i, j, k)] = nid
        x, y, z = (c * s / st for c, s, st in zip((i, j, k), size, steps))
        lines.append(f"{nid}, {x:.6g}, {y:.6g}, {z:.6g}")
    return ids, lines


def node_set(name, ids, predicate):
    members = sorted(nid for key, nid in ids.items() if predicate(key))
    rows = [", ".join(map(str, members[i:i + 10])) for i in range(0, len(members), 10)]
    return f"*NSET, NSET={name}\n" + "\n".join(rows) + "\n"


def cantilever_c3d8():
    nx, ny, nz = 10, 2, 2
    ids, nodes = grid(nx, ny, nz, (100.0, 10.0, 10.0), 1)
    elements = []
    for k, j, i in itertools.product(range(nz), range(ny), range(nx)):
        c = [(i, j, k), (i + 1, j, k), (i + 1, j + 1, k), (i, j + 1, k)]
        c += [(a, b, k + 1) for a, b, _ in c]
        elements.append(f"{len(elements) + 1}, " + ", ".join(str(ids[p]) for p in c))
    tip = ids[(nx, ny, nz)]
    text = "*HEADING\nKragbalken aus C3D8-Elementen\n*NODE, NSET=NALL\n" + "\n".join(nodes) + "\n"
    text += "*ELEMENT, TYPE=C3D8, ELSET=EALL\n" + "\n".join(elements) + "\n"
    text += node_set("FIX", ids, lambda p: p[0] == 0)
    text += "*ELSET, ELSET=TIP_ELEMENTS, GENERATE\n10, 40, 10\n"
    text += "*SURFACE, NAME=TIP, TYPE=ELEMENT\nTIP_ELEMENTS, S4\n"
    text += STEEL_STEP.format(load=f"{tip}, 3, -100.")
    write("kragbalken_c3d8.inp", text)


def block_c3d20r():
    nx, ny, nz = 4, 2, 2
    ids, nodes = grid(nx, ny, nz, (40.0, 20.0, 20.0), 2)
    elements = []
    for k, j, i in itertools.product(range(nz), range(ny), range(nx)):
        i2, j2, k2 = 2 * i, 2 * j, 2 * k
        corners = [(i2, j2, k2), (i2 + 2, j2, k2), (i2 + 2, j2 + 2, k2), (i2, j2 + 2, k2)]
        corners += [(a, b, k2 + 2) for a, b, _ in corners]
        mid = lambda p, q: tuple((a + b) // 2 for a, b in zip(p, q))
        edges = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)]
        pts = corners + [mid(corners[a], corners[b]) for a, b in edges]
        numbers = [len(elements) + 1] + [ids[p] for p in pts]
        first, rest = numbers[:16], numbers[16:]
        elements.append(", ".join(map(str, first)) + ",\n" + ", ".join(map(str, rest)))
    tip = ids[(2 * nx, 2 * ny, 2 * nz)]
    text = "*NODE\n" + "\n".join(nodes) + "\n"
    text += "*ELEMENT, TYPE=C3D20R, ELSET=EALL\n" + "\n".join(elements) + "\n"
    text += node_set("FIX", ids, lambda p: p[0] == 0)
    text += STEEL_STEP.format(load=f"{tip}, 2, -500.")
    write("block_c3d20r.inp", text)


def cantilever_c3d20r():
    """Kragbalken 100 x 10 x 10 aus C3D20R, Querlast am Ende auf alle Knoten verteilt.

    Biegespannung oben nach Balkentheorie: S11 = F (100 - x) * 5 / 833.3 = 0.6 (100 - x).
    """
    nx, ny, nz = 20, 2, 2
    ids, nodes = grid(nx, ny, nz, (100.0, 10.0, 10.0), 2)
    elements = []
    for k, j, i in itertools.product(range(nz), range(ny), range(nx)):
        i2, j2, k2 = 2 * i, 2 * j, 2 * k
        corners = [(i2, j2, k2), (i2 + 2, j2, k2), (i2 + 2, j2 + 2, k2), (i2, j2 + 2, k2)]
        corners += [(a, b, k2 + 2) for a, b, _ in corners]
        mid = lambda p, q: tuple((a + b) // 2 for a, b in zip(p, q))
        edges = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)]
        pts = corners + [mid(corners[a], corners[b]) for a, b in edges]
        numbers = [len(elements) + 1] + [ids[p] for p in pts]
        first, rest = numbers[:16], numbers[16:]
        elements.append(", ".join(map(str, first)) + ",\n" + ", ".join(map(str, rest)))
    tip = [nid for key, nid in ids.items() if key[0] == 2 * nx]
    text = "*HEADING\nKragbalken aus C3D20R-Elementen\n*NODE, NSET=NALL\n" + "\n".join(nodes) + "\n"
    text += "*ELEMENT, TYPE=C3D20R, ELSET=EALL\n" + "\n".join(elements) + "\n"
    text += node_set("FIX", ids, lambda p: p[0] == 0)
    text += node_set("TIP", ids, lambda p: p[0] == 2 * nx)
    text += STEEL_STEP.format(load=f"TIP, 3, {-100.0 / len(tip):.10g}")
    write("kragbalken_c3d20r.inp", text)


def tets_c3d10():
    """Würfelgitter, jeder Würfel in 6 Tetraeder entlang der Raumdiagonale zerlegt."""
    nx, ny, nz = 3, 3, 3
    coords, ids = {}, {}

    def node(p):
        if p not in ids:
            ids[p] = len(ids) + 1
            coords[p] = tuple(c * 10.0 / 2 for c in p)
        return ids[p]

    paths = list(itertools.permutations(range(3)))
    elements = []
    for i, j, k in itertools.product(range(nx), range(ny), range(nz)):
        base = (2 * i, 2 * j, 2 * k)
        for perm in paths:
            pts = [base]
            p = list(base)
            for axis in perm:
                p[axis] += 2
                pts.append(tuple(p))
            v = [[b - a for a, b in zip(pts[0], q)] for q in pts[1:]]
            det = (v[0][0] * (v[1][1] * v[2][2] - v[1][2] * v[2][1])
                   - v[0][1] * (v[1][0] * v[2][2] - v[1][2] * v[2][0])
                   + v[0][2] * (v[1][0] * v[2][1] - v[1][1] * v[2][0]))
            if det < 0:
                pts[1], pts[2] = pts[2], pts[1]
            mid = lambda a, b: tuple((x + y) // 2 for x, y in zip(pts[a], pts[b]))
            all_pts = pts + [mid(0, 1), mid(1, 2), mid(2, 0), mid(0, 3), mid(1, 3), mid(2, 3)]
            elements.append(f"{len(elements) + 1}, " + ", ".join(str(node(q)) for q in all_pts))
    lines = [f"{n}, {coords[p][0]:.6g}, {coords[p][1]:.6g}, {coords[p][2]:.6g}"
             for p, n in sorted(ids.items(), key=lambda item: item[1])]
    tip = ids[(2 * nx, 2 * ny, 2 * nz)]
    text = "*NODE\n" + "\n".join(lines) + "\n"
    text += "*ELEMENT, TYPE=C3D10, ELSET=EALL\n" + "\n".join(elements) + "\n"
    text += node_set("FIX", ids, lambda p: p[2] == 0)
    text += STEEL_STEP.format(load=f"{tip}, 1, 200.")
    write("wuerfel_c3d10.inp", text)


def mixed_parts():
    """Platte aus S4R-Schalen mit zwei B31-Balken; Knoten liegen in einer eingebundenen Datei."""
    n = 4
    nodes, ids = [], {}
    for j, i in itertools.product(range(n + 1), range(n + 1)):
        ids[(i, j)] = len(ids) + 1
        nodes.append(f"{ids[(i, j)]}, {i * 5.0:.6g}, {j * 5.0:.6g}, 0")
    top = len(ids)
    for k, (i, j) in enumerate([(0, 0), (n, 0)]):
        nodes.append(f"{top + k + 1}, {i * 5.0:.6g}, {j * 5.0:.6g}, -15")
    shells = [f"{len(ids) * 0 + e + 1}, {ids[(i, j)]}, {ids[(i + 1, j)]}, {ids[(i + 1, j + 1)]}, {ids[(i, j + 1)]}"
              for e, (j, i) in enumerate(itertools.product(range(n), range(n)))]
    beams = [f"{101 + k}, {top + k + 1}, {ids[p]}" for k, p in enumerate([(0, 0), (n, 0)])]
    write("platte_knoten.inp", "*NODE, NSET=NALL\n" + "\n".join(nodes) + "\n")
    text = "*INCLUDE, INPUT=platte_knoten.inp\n"
    text += "*ELEMENT, TYPE=S4R, ELSET=PLATTE\n" + "\n".join(shells) + "\n"
    text += "*ELEMENT, TYPE=B31, ELSET=STUETZEN\n" + "\n".join(beams) + "\n"
    text += f"*NSET, NSET=FUESSE\n{top + 1}, {top + 2}\n"
    write("platte_mit_stuetzen.inp", text)


if __name__ == "__main__":
    cantilever_c3d8()
    cantilever_c3d20r()
    block_c3d20r()
    tets_c3d10()
    mixed_parts()
