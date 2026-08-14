"""P53.9 — KuzuDB drop-in compat harness.

Runs the REAL Kairos stores — ``KuzuDBStore`` and ``KuzuDBDreamBackend``
(``kairos/kuzudb_store.py``, ``kairos/dream_kuzudb_store.py``) — against
``import akar`` instead of ``import kuzu``, via module aliases registered in
``sys.modules`` before any kairos module is imported. Every test uses a fresh
temp database so failures are isolated and map 1:1 to a store method.

Purpose: capture the gaps that block a one-line ``import kuzu`` →
``import akar`` drop-in, method by method. The assertions encode the expected
drop-in behaviour; each failing test is a gap. On any failure a machine
readable ``test_kuzu_compat.gap_report.json`` is written next to this file.

Run (after ``maturin develop`` once in ``akar-core/akar-python``)::

    .venv\\Scripts\\python -m pytest tests/test_kuzu_compat.py -v

Set ``KAIROS_SRC`` if the kairos repo is not the sibling of this repo root.
"""

from __future__ import annotations

import os
import sys
import types
from pathlib import Path

import pytest

# ---------------------------------------------------------------------------
# module alias: `import kuzu` / `import ladybug` → akar
# ---------------------------------------------------------------------------
try:
    import akar  # noqa: PLC0415
except ImportError as exc:  # pragma: no cover - environment only
    raise RuntimeError(
        "`import akar` failed. Build once with `maturin develop` in "
        "akar-core/akar-python and re-run pytest from that venv."
    ) from exc


def _install_aliases() -> None:
    for mod_name in ("kuzu", "ladybug"):
        mod = types.ModuleType(mod_name)
        mod.Database = akar.Database
        mod.Connection = akar.Connection
        mod.QueryResult = akar.QueryResult
        sys.modules[mod_name] = mod


_install_aliases()

# ---------------------------------------------------------------------------
# locate + import the real kairos stores
# ---------------------------------------------------------------------------
_KAIROS_SRC = os.environ.get("KAIROS_SRC", "")
_KAIROS = Path(_KAIROS_SRC).expanduser() if _KAIROS_SRC else None
if _KAIROS is None or not _KAIROS.is_dir():
    _KAIROS = Path(__file__).resolve().parents[4] / "kairos"
if not _KAIROS.is_dir():  # pragma: no cover - environment only
    pytest.skip(
        "kairos repo not found (set KAIROS_SRC or place it at the repo sibling)",
        allow_module_level=True,
    )

sys.path.insert(0, str(_KAIROS))
from kairos.dream_kuzudb_store import KuzuDBDreamBackend  # noqa: E402, PLC0415
from kairos.kuzudb_store import KuzuDBStore  # noqa: E402, PLC0415

DIM = 8


def _embed(i: int) -> list[float]:
    """Deterministic 8-dim unit-ish vector; i'th coord biased toward 1."""
    vec = [0.0] * DIM
    vec[i % DIM] = 1.0
    if i // DIM:
        vec[(i + 2) % DIM] = 0.5
    return vec


# ---------------------------------------------------------------------------
# gap capture: failures collected by tests/conftest.py into
# test_kuzu_compat.gap_report.json at session end.
# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# fixtures
# ---------------------------------------------------------------------------
@pytest.fixture
def store(tmp_path: Path) -> KuzuDBStore:
    s = KuzuDBStore(db_path=str(tmp_path / "mem.db"), dimension=DIM)
    yield s
    s.close()


@pytest.fixture
def dream(store: KuzuDBStore) -> KuzuDBDreamBackend:
    d = KuzuDBDreamBackend(store=store)
    yield d
    d.close()


@pytest.fixture
def seeded_store(store: KuzuDBStore) -> KuzuDBStore:
    """Store 3 memories with explicit ids + one Connected edge via raw DML.

    Bypasses id allocation (MERGE ... SET counter) and connection writes so
    read/search paths can be exercised in isolation.
    """
    for i in (1, 2, 3):
        store._execute(
            "CREATE (m:Memory {id: $id, label: $label, content: $content, "
            "embedding: $emb, salience: 1.0, content_hash: '', "
            "session_id: '', prof: '', scope: 'shared', "
            "created_at: 1000.0, last_accessed: 1000.0, access_count: 0})",
            {"id": i, "label": f"mem{i}", "content": f"hello world {i}", "emb": _embed(i)},
        )
    store._execute(
        "MATCH (a:Memory {id: 1}), (b:Memory {id: 2}) "
        "CREATE (a)-[r:Connected {weight: 0.9, type: 'similar', "
        "created_at: 1000.0, event_time: 1000.0, ingestion_time: 1000.0, "
        "valid_from: 1000.0}]->(b)"
    )
    return store


# ===========================================================================
# KuzuDBStore
# ===========================================================================


class TestSchemaBootstrap:
    def test_core_tables_exist(self, store: KuzuDBStore) -> None:
        rows = store._query_all("CALL show_tables()")
        kinds = {str(r.get("col_1") or r.get("name") or "") for r in rows}
        names = {str(r.get("col_0") or r.get("name") or "") for r in rows}
        for expected in ("Memory", "Connected", "Revision", "Meta", "Counter"):
            assert expected in names, f"table {expected} missing; tables={sorted(names)}"
        assert "NODE" in kinds and "REL" in kinds

    def test_metadata_tables_exist(self, store: KuzuDBStore) -> None:
        rows = store._query_all("CALL show_tables()")
        names = {str(r.get("col_0") or r.get("name") or "") for r in rows}
        for expected in ("Database", "Schema", "MetadataTable", "MetadataColumn"):
            assert expected in names, f"metadata table {expected} missing; tables={sorted(names)}"

    def test_bootstrap_is_idempotent(self, store: KuzuDBStore) -> None:
        store._ensure_schema()
        rows = store._query_all("CALL show_tables()")
        names = {str(r.get("col_0") or r.get("name") or "") for r in rows}
        assert "Memory" in names and "Counter" in names

    def test_metadata_rel_tables_exist(self, store: KuzuDBStore) -> None:
        rows = store._query_all("CALL show_tables()")
        names = {str(r.get("col_0") or r.get("name") or "") for r in rows}
        for expected in ("HAS_SCHEMA", "HAS_TABLE", "HAS_COLUMN"):
            assert expected in names, f"rel table {expected} missing; tables={sorted(names)}"


class TestStore:
    def test_store_and_get(self, store: KuzuDBStore) -> None:
        mid = store.store("hello", "world hello content", _embed(1), content_hash="h-1")
        assert mid == 1
        m = store.get(mid)
        assert m is not None
        assert m["id"] == 1
        assert m["label"] == "hello"
        assert m["content"] == "world hello content"
        assert len(m["embedding"]) == DIM

    def test_store_with_explicit_id(self, store: KuzuDBStore) -> None:
        mid = store.store("a", "content a", _embed(2), id_=42, content_hash="h-42")
        assert mid == 42
        m = store.get(42)
        assert m is not None and m["id"] == 42

    def test_store_dedup_by_content_hash(self, store: KuzuDBStore) -> None:
        first = store.store("a", "same content", _embed(1), id_=7, content_hash="dup")
        second = store.store("a", "same content", _embed(1), content_hash="dup")
        assert first == second == 7
        m = store.get(7)
        assert m is not None and m["access_count"] == 1

    def test_store_many(self, store: KuzuDBStore) -> None:
        ids = store.store_many(
            [
                {"label": "x", "content": "one", "embedding": _embed(1)},
                {"label": "y", "content": "two", "embedding": _embed(2)},
                {"label": "z", "content": "three", "embedding": _embed(3), "id_": 50},
            ]
        )
        assert len(ids) == 3
        assert ids[0] == 1 and ids[1] == 2 and ids[2] == 50
        fetched = store.get_many(ids, include_embedding=False)
        assert set(fetched) == {1, 2, 50}

    def test_get_missing_returns_none(self, store: KuzuDBStore) -> None:
        assert store.get(9999) is None

    def test_get_many(self, seeded_store: KuzuDBStore) -> None:
        got = seeded_store.get_many([1, 2, 3], include_embedding=True)
        assert set(got) == {1, 2, 3}
        assert got[2]["content"] == "hello world 2"
        assert len(got[1]["embedding"]) == DIM

    def test_get_all(self, seeded_store: KuzuDBStore) -> None:
        rows = seeded_store.get_all()
        assert len(rows) == 3
        assert [r["id"] for r in rows] == [1, 2, 3]
        assert all(len(r["embedding"]) == DIM for r in rows)

    def test_list_ids(self, seeded_store: KuzuDBStore) -> None:
        assert seeded_store.list_ids() == [1, 2, 3]

    def test_find_by_content_hash(self, store: KuzuDBStore) -> None:
        store.store("a", "c", _embed(1), id_=9, content_hash="unique-hash")
        assert store.find_by_content_hash("unique-hash") == 9
        assert store.find_by_content_hash("nope") is None

    def test_find_by_label(self, seeded_store: KuzuDBStore) -> None:
        rows = seeded_store.find_by_label("mem2")
        assert len(rows) == 1 and rows[0]["id"] == 2

    def test_get_max_id(self, seeded_store: KuzuDBStore) -> None:
        assert seeded_store.get_max_id() == 3

    def test_count(self, seeded_store: KuzuDBStore) -> None:
        assert seeded_store.count() == 3

    def test_revision_count(self, store: KuzuDBStore) -> None:
        assert store.revision_count() == 0

    def test_add_revision(self, store: KuzuDBStore) -> None:
        store.store("a", "old", _embed(1), id_=1)
        store.add_revision(1, "old", "new", reason="conflict_fusion")
        assert store.revision_count() == 1


class TestConnections:
    def test_add_connection(self, seeded_store: KuzuDBStore) -> None:
        seeded_store.add_connection(1, 3, weight=0.7, edge_type="similar")
        conns = seeded_store.get_connections(1)
        assert any(c["target"] == 3 and c["weight"] == 0.7 for c in conns)

    def test_add_connections_batch(self, seeded_store: KuzuDBStore) -> None:
        seeded_store.add_connections_batch([(1, 3), (2, 3)])
        conns = seeded_store.get_connections(1)
        assert any(c["target"] == 3 for c in conns)
        assert seeded_store.connection_count() == 3

    def test_get_connections(self, seeded_store: KuzuDBStore) -> None:
        conns = seeded_store.get_connections(1)
        assert len(conns) == 1
        assert conns[0]["target"] == 2
        assert conns[0]["weight"] == 0.9
        assert conns[0]["type"] == "similar"

    def test_get_connections_at_time(self, seeded_store: KuzuDBStore) -> None:
        conns = seeded_store.get_connections(1, at_time=2000.0)
        assert len(conns) == 1 and conns[0]["target"] == 2

    def test_get_all_connections(self, seeded_store: KuzuDBStore) -> None:
        conns = seeded_store.get_all_connections()
        assert len(conns) == 1 and conns[0]["target"] == 2

    def test_top_weighted_edges(self, seeded_store: KuzuDBStore) -> None:
        edges = seeded_store.top_weighted_edges(10)
        assert len(edges) == 1 and edges[0]["weight"] == 0.9

    def test_connection_count(self, seeded_store: KuzuDBStore) -> None:
        assert seeded_store.connection_count() == 1


class TestUpdateAndMetadata:
    def test_update_memory_content(self, seeded_store: KuzuDBStore) -> None:
        seeded_store.update_memory(1, "brand new content", _embed(1))
        m = seeded_store.get(1)
        assert m is not None and m["content"] == "brand new content"

    def test_touch(self, seeded_store: KuzuDBStore) -> None:
        seeded_store.touch(1)
        m = seeded_store.get(1)
        assert m is not None and m["access_count"] == 1

    def test_set_meta_get_meta(self, store: KuzuDBStore) -> None:
        store.set_meta("k", "v")
        assert store.get_meta("k") == "v"


class TestSearch:
    def test_search_semantic_returns_candidates(self, seeded_store: KuzuDBStore) -> None:
        results = seeded_store.search_semantic(_embed(1), limit=10)
        assert len(results) >= 1
        assert results[0]["channel"] == "semantic"
        assert results[0]["id"] in (1, 2, 3)

    def test_search_semantic_similarity_score(self, seeded_store: KuzuDBStore) -> None:
        results = seeded_store.search_semantic(_embed(1), limit=10)
        assert results and results[0]["id"] == 1
        assert results[0]["similarity"] > 0.0

    def test_search_bm25(self, seeded_store: KuzuDBStore) -> None:
        results = seeded_store.search_bm25("hello", limit=10)
        assert len(results) == 3
        assert all(r["channel"] == "bm25" for r in results)

    def test_search_entity(self, seeded_store: KuzuDBStore) -> None:
        results = seeded_store.search_entity("mem2", limit=10)
        assert len(results) == 1 and results[0]["id"] == 2

    def test_search_temporal(self, seeded_store: KuzuDBStore) -> None:
        results = seeded_store.search_temporal("hello", limit=10)
        assert len(results) == 3
        assert all(r["created_at"] == 1000.0 for r in results)


class TestMaintenance:
    def test_export_database(self, store: KuzuDBStore, tmp_path: Path) -> None:
        store.store("a", "c", _embed(1), id_=1)
        out = str(tmp_path / "backup")
        result = store.export_database(out)
        assert result["ok"] is True
        assert Path(out).is_dir()

    def test_repair_schema(self, seeded_store: KuzuDBStore) -> None:
        stats = seeded_store.repair_schema()
        assert stats["ok"] is True, stats
        assert seeded_store.count() == 3

    def test_close_and_reopen(self, store: KuzuDBStore, tmp_path: Path) -> None:
        path = str(tmp_path / "mem.db")
        s = KuzuDBStore(db_path=path, dimension=DIM)
        s.store("a", "persist me", _embed(1), id_=1)
        s.close()
        s2 = KuzuDBStore(db_path=path, dimension=DIM)
        try:
            m = s2.get(1)
            assert m is not None and m["content"] == "persist me"
        finally:
            s2.close()


# ===========================================================================
# KuzuDBDreamBackend
# ===========================================================================


class TestDream:
    def test_dream_schema_tables_exist(self, dream: KuzuDBDreamBackend) -> None:
        rows = dream._query_all("CALL show_tables()")
        names = {str(r.get("col_0") or r.get("name") or "") for r in rows}
        for expected in ("DreamSession", "ConnectionHistory", "DreamInsight"):
            assert expected in names, f"dream table {expected} missing; tables={sorted(names)}"

    def test_start_finish_session(self, dream: KuzuDBDreamBackend) -> None:
        sid = dream.start_session("nrem")
        assert sid >= 1
        dream.finish_session(sid, {"processed": 5, "strengthened": 2})

    def test_get_dream_stats(self, dream: KuzuDBDreamBackend) -> None:
        stats = dream.get_dream_stats()
        assert stats["sessions"] == 0

    def test_add_insight(self, dream: KuzuDBDreamBackend) -> None:
        sid = 1
        dream.add_insight(sid, "cluster", 2, "a cluster insight", 0.9)

    def test_add_insights_batch(self, dream: KuzuDBDreamBackend) -> None:
        n = dream.add_insights_batch([(1, "cluster", 2, "insight A", 0.9), (1, "pattern", 3, "insight B", 0.8)])
        assert n == 2

    def test_log_connection_change(self, seeded_store: KuzuDBStore, dream: KuzuDBDreamBackend) -> None:
        dream.log_connection_change(1, 2, 0.5, 0.8, "nrem_strengthen", 7)

    def test_strengthen_connection(self, seeded_store: KuzuDBStore, dream: KuzuDBDreamBackend) -> None:
        dream.strengthen_connection(1, 2, delta=0.05)
        conns = seeded_store.get_connections(1)
        assert conns[0]["weight"] == 0.95

    def test_batch_strengthen_connections(self, seeded_store: KuzuDBStore, dream: KuzuDBDreamBackend) -> None:
        n = dream.batch_strengthen_connections([(1, 2)], delta=0.05, dream_session_id=7)
        assert n == 1
        assert seeded_store.get_connections(1)[0]["weight"] == 0.95

    def test_set_connection_weight(self, seeded_store: KuzuDBStore, dream: KuzuDBDreamBackend) -> None:
        assert dream.set_connection_weight(1, 2, 0.25) is True
        assert seeded_store.get_connections(1)[0]["weight"] == 0.25

    def test_add_bridge(self, seeded_store: KuzuDBStore, dream: KuzuDBDreamBackend) -> None:
        assert dream.add_bridge(1, 3, weight=0.3) is True
        targets = {c["target"] for c in seeded_store.get_connections(1)}
        assert 3 in targets

    def test_add_supersedes_batch(self, seeded_store: KuzuDBStore, dream: KuzuDBDreamBackend) -> None:
        n = dream.add_supersedes_batch([(1, 3, 0.5)])
        assert n == 1

    def test_prune_weak(self, seeded_store: KuzuDBStore, dream: KuzuDBDreamBackend) -> None:
        dream.set_connection_weight(1, 2, 0.02)
        pruned = dream.prune_weak(threshold=0.05)
        assert pruned == 1
        assert seeded_store.connection_count() == 0

    def test_prune_connection_history(self, dream: KuzuDBDreamBackend) -> None:
        assert dream.prune_connection_history(keep_days=7) == 0

    def test_recent_cluster_anchors(self, seeded_store: KuzuDBStore, dream: KuzuDBDreamBackend) -> None:
        dream.add_insight(1, "cluster", 2, "c", 0.9)
        anchors = dream.recent_cluster_anchors(window_seconds=3600)
        assert 2 in anchors

    def test_close(self, dream: KuzuDBDreamBackend) -> None:
        dream.close()
