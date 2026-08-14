import os
import shutil
import sys

sys.path.insert(0, os.environ.get("SMOKE_VENV", ""))

import akar

DB_PATH = os.path.join(os.environ.get("TEMP", "."), "akar_smoke_test.db")
if os.path.exists(DB_PATH):
    shutil.rmtree(DB_PATH)

db = akar.Database(DB_PATH)
conn = akar.Connection(db)

# 1. Multi-statement bootstrap: INSTALL/LOAD + CREATE NODE TABLE IF NOT EXISTS
bootstrap = """
INSTALL vector;
LOAD EXTENSION vector;
CREATE NODE TABLE IF NOT EXISTS Memory (
    id INT64,
    content STRING,
    embedding FLOAT[384],
    salience DOUBLE,
    PRIMARY KEY (id)
);
CREATE NODE TABLE IF NOT EXISTS `Database` (name STRING, PRIMARY KEY (name));
CREATE NODE TABLE IF NOT EXISTS `Schema` (name STRING, PRIMARY KEY (name));
CREATE REL TABLE IF NOT EXISTS HAS_SCHEMA (FROM `Database` TO `Schema`);
"""
r = conn.execute(bootstrap)
assert r is not None, "bootstrap failed"
print("step1 bootstrap ok", flush=True)

# Idempotency: running the same bootstrap again must not error.
r = conn.execute(bootstrap)
print("step1b bootstrap idempotent ok", flush=True)

# 2. CALL CREATE_VECTOR_INDEX -> CREATE VECTOR INDEX (dims from FLOAT[384])
r = conn.execute("CALL CREATE_VECTOR_INDEX('Memory', 'mem_vec', 'embedding', metric := 'cosine')")
print("step2 create idx ok", flush=True)
r = conn.execute("CALL CREATE_VECTOR_INDEX('Memory', 'mem_vec', 'embedding', metric := 'cosine')")
print("step2b create idx idempotent ok", flush=True)

# 3. INSERT a few rows (via params interpolation)
def embed(n):
    v = [0.0] * 384
    v[0] = float(n)
    v[1] = 1.0 - n * 0.1
    return v

for i, content in enumerate(["hello world", "goodbye world", "foo bar", "baz qux"]):
    conn.execute(
        "CREATE (:Memory {id: $id, content: $content, embedding: $vec, salience: $sal})",
        {"id": i, "content": content, "vec": embed(i), "sal": 0.5 + i * 0.1},
    )
print("step3 inserts ok", flush=True)

# 4. QUERY_VECTOR_INDEX translation -> brute-force MATCH, params interpolated
q = conn.execute(
    "CALL QUERY_VECTOR_INDEX('Memory', 'mem_vec', $query_vec, $limit) RETURN node, distance",
    {"query_vec": embed(0), "limit": 2},
)
assert q is not None, "query failed"
q.rows_as_dict(True)
rows = q.get_all()
assert len(rows) == 2, f"expected 2 rows, got {len(rows)}: {rows}"
first = rows[0]
assert "node" in first and "distance" in first, f"columns wrong: {first}"
node = first["node"]
assert isinstance(node, dict), f"node should be a dict, got {type(node)}: {node}"
assert "content" in node and "id" in node and "embedding" in node, f"node props wrong: {node}"
assert node["content"] == "hello world", f"nearest should be hello world, got {node}"
assert abs(first["distance"] - 1.0) < 1e-6, f"distance should be ~1.0, got {first['distance']}"
print("step4 vector query ok", flush=True)

# 5. WHERE clause passthrough + param filter
q2 = conn.execute(
    "CALL QUERY_VECTOR_INDEX('Memory', 'mem_vec', $q, $k) RETURN node, distance WHERE (node.salience > $min)",
    {"q": embed(3), "k": 5, "min": 0.7},
)
q2.rows_as_dict(True)
rows2 = q2.get_all()
assert 0 < len(rows2) <= 5, f"bad filtered rows: {rows2}"
print("step5 where filter ok", flush=True)

# 6. ALTER TABLE ADD ... DEFAULT <lit> (swallow already-exists)
conn.execute("ALTER TABLE Memory ADD protected BOOLEAN DEFAULT false")
conn.execute("ALTER TABLE Memory ADD protected BOOLEAN DEFAULT false")

# 7. DROP TABLE IF EXISTS (swallow not found)
conn.execute("DROP TABLE IF EXISTS DoesNotExist")

# 8. Kuzu CREATE VECTOR INDEX ... FOR/ON/OPTIONS syntax
conn.execute(
    "CREATE VECTOR INDEX IF NOT EXISTS mem2 FOR (m:Memory) ON (m.embedding) OPTIONS {index_list: [{efc: 128, M: 16}]}"
)

# 9. ALTER after DROP VECTOR INDEX
conn.execute("CALL DROP_VECTOR_INDEX('Memory', 'mem2')")
conn.execute("CALL DROP_VECTOR_INDEX('Memory', 'mem2')")

# 10. Reopen database: schema must be recovered from catalog (fallback table_info)
db.close()
db2 = akar.Database(DB_PATH)
conn2 = akar.Connection(db2)
q3 = conn2.execute(
    "CALL QUERY_VECTOR_INDEX('Memory', 'mem_vec', $query_vec, $limit) RETURN node, distance",
    {"query_vec": embed(0), "limit": 1},
)
q3.rows_as_dict(True)
rows3 = q3.get_all()
assert len(rows3) == 1, f"expected 1 row after reopen, got {rows3}"
assert rows3[0]["node"]["content"] == "hello world", rows3

print("SMOKE_OK")
