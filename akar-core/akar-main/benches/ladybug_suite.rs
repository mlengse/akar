use akar_main::connection::Connection;
use akar_main::database::{Database, SystemConfig};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;

static BENCH_DIR: OnceLock<TempDir> = OnceLock::new();
static BENCH_DB_NODES: OnceLock<Arc<Database>> = OnceLock::new();
static BENCH_DB_FULL: OnceLock<Arc<Database>> = OnceLock::new();
static BENCH_DB_100K: OnceLock<Arc<Database>> = OnceLock::new();
static BENCH_DB_1M: OnceLock<Arc<Database>> = OnceLock::new();
static BENCH_DB_WCOJ_FAN: OnceLock<Arc<Database>> = OnceLock::new();
static BENCH_DB_WCOJ_TRIANGLE: OnceLock<Arc<Database>> = OnceLock::new();

fn setup_nodes_db(dir: &TempDir) -> Arc<Database> {
    let db_path = dir.path().join("nodes_only");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query(
        "CREATE NODE TABLE Person(id INT64, name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (id))",
    )
    .unwrap();
    conn.query("CREATE NODE TABLE City(name STRING, population INT64, country STRING, PRIMARY KEY (name))")
        .unwrap();

    let csv_path = dir.path().join("person_10k.csv");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&csv_path).unwrap();
        writeln!(f, "id,name,age,score,active").unwrap();
        for i in 0..10_000u64 {
            let age = (i * 7 + 13) % 101;
            let score = (i as f64) * 0.01 + 50.0;
            let active = i % 3 != 0;
            writeln!(f, "{i},P{i},{age},{score},{active}").unwrap();
        }
    }
    let fp = csv_path.to_string_lossy().replace('\\', "/");
    conn.query(&format!("COPY Person FROM '{fp}' (HEADER true)")).unwrap();

    let cities = [
        "New York",
        "London",
        "Tokyo",
        "Paris",
        "Berlin",
        "Sydney",
        "Toronto",
        "Mumbai",
        "Sao Paulo",
        "Cairo",
    ];
    for (i, city) in cities.iter().enumerate() {
        let pop = 1_000_000 + (i as i64) * 500_000;
        conn.query(&format!(
            "CREATE (c:City {{name: '{city}', population: {pop}, country: 'X'}})"
        ))
        .unwrap();
    }
    database
}

fn setup_full_db(dir: &TempDir) -> Arc<Database> {
    let db_path = dir.path().join("full_bench");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query(
        "CREATE NODE TABLE Person(id INT64, name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (id))",
    )
    .unwrap();
    conn.query("CREATE NODE TABLE City(id INT64, name STRING, population INT64, country STRING, PRIMARY KEY (name))")
        .unwrap();
    conn.query("CREATE REL TABLE Knows(FROM Person TO Person, since INT64)")
        .unwrap();
    conn.query("CREATE REL TABLE LivesIn(FROM Person TO City, since INT64)")
        .unwrap();

    let csv_path = dir.path().join("person_10k.csv");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&csv_path).unwrap();
        writeln!(f, "id,name,age,score,active").unwrap();
        for i in 0..10_000u64 {
            let age = (i * 7 + 13) % 101;
            let score = (i as f64) * 0.01 + 50.0;
            let active = i % 3 != 0;
            writeln!(f, "{i},P{i},{age},{score},{active}").unwrap();
        }
    }
    let fp = csv_path.to_string_lossy().replace('\\', "/");
    conn.query(&format!("COPY Person FROM '{fp}' (HEADER true)")).unwrap();

    let cities = [
        "New York",
        "London",
        "Tokyo",
        "Paris",
        "Berlin",
        "Sydney",
        "Toronto",
        "Mumbai",
        "Sao Paulo",
        "Cairo",
    ];
    for (i, city) in cities.iter().enumerate() {
        let pop = 1_000_000 + (i as i64) * 500_000;
        conn.query(&format!(
            "CREATE (c:City {{id: {i}, name: '{city}', population: {pop}, country: 'X'}})"
        ))
        .unwrap();
    }

    for i in 0..5000u64 {
        let target = (i + 1) % 10_000;
        conn.query(&format!(
            "MATCH (a:Person {{id: {i}}}), (b:Person {{id: {target}}}) CREATE (a)-[:Knows {{since: 2020}}]->(b)"
        ))
        .unwrap();
    }
    for i in 0..10_000u64 {
        let city_idx = i % 10;
        let city = cities[city_idx as usize];
        conn.query(&format!(
            "MATCH (p:Person {{id: {i}}}), (c:City {{name: '{city}'}}) CREATE (p)-[:LivesIn {{since: 2015}}]->(c)"
        ))
        .unwrap();
    }

    database
}

fn get_nodes_db() -> Connection {
    let dir = BENCH_DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let db = BENCH_DB_NODES.get_or_init(|| setup_nodes_db(dir));
    Connection::new(db)
}

fn get_full_db() -> Connection {
    let dir = BENCH_DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let db = BENCH_DB_FULL.get_or_init(|| setup_full_db(dir));
    Connection::new(db)
}

fn setup_100k_db(dir: &TempDir) -> Arc<Database> {
    let db_path = dir.path().join("bench_100k");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query(
        "CREATE NODE TABLE Person(id INT64, name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (id))",
    )
    .unwrap();

    let csv_path = dir.path().join("person_100k.csv");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&csv_path).unwrap();
        writeln!(f, "id,name,age,score,active").unwrap();
        for i in 0..100_000u64 {
            let age = (i * 7 + 13) % 101;
            let score = (i as f64) * 0.001 + 50.0;
            let active = i % 3 != 0;
            writeln!(f, "{i},P{i},{age},{score},{active}").unwrap();
        }
    }
    let fp = csv_path.to_string_lossy().replace('\\', "/");
    conn.query(&format!("COPY Person FROM '{fp}' (HEADER true)")).unwrap();

    database
}

fn setup_1m_db(dir: &TempDir) -> Arc<Database> {
    let db_path = dir.path().join("bench_1m");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query(
        "CREATE NODE TABLE Person(id INT64, name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (id))",
    )
    .unwrap();

    let csv_path = dir.path().join("person_1m.csv");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&csv_path).unwrap();
        writeln!(f, "id,name,age,score,active").unwrap();
        for i in 0..1_000_000u64 {
            let age = (i * 7 + 13) % 101;
            let score = (i as f64) * 0.0001 + 50.0;
            let active = i % 3 != 0;
            writeln!(f, "{i},P{i},{age},{score},{active}").unwrap();
        }
    }
    let fp = csv_path.to_string_lossy().replace('\\', "/");
    conn.query(&format!("COPY Person FROM '{fp}' (HEADER true)")).unwrap();

    database
}

fn get_100k_db() -> Connection {
    let dir = BENCH_DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let db = BENCH_DB_100K.get_or_init(|| setup_100k_db(dir));
    Connection::new(db)
}

fn get_1m_db() -> Connection {
    let dir = BENCH_DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let db = BENCH_DB_1M.get_or_init(|| setup_1m_db(dir));
    Connection::new(db)
}

/// P48.4 fan DB — WCOJ star vs HashJoin chain on the same tables.
///
/// Person 151 (id 0..=150), Tag 101 (id 0..=100). Edges built with bulk
/// WHERE-comparison CREATE:
///   - `r1`: Person -> Person. For each center `a` in 0..100, edges to the 10
///     Persons `(a, a+10]` (1000 edges).
///   - `r2`: Person -> Tag. For each center `a` in 0..100, edges to Tags `[0,9]`
///     (1000 edges). Star through a center = 10x10 = 100 rows each => 10k rows.
///   - `r3t`: Person -> Tag. For each Person `b` in 0..150, edges to Tags
///     `[0,9]` (1510 edges). Chain `r1`-then-`r3t` = 1000 x 10 = 10k rows.
fn setup_wcoj_fan_db(dir: &TempDir) -> Arc<Database> {
    let db_path = dir.path().join("wcoj_fan");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query("CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE NODE TABLE Tag(id INT64, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE REL TABLE r1(FROM Person TO Person)").unwrap();
    conn.query("CREATE REL TABLE r2(FROM Person TO Tag)").unwrap();
    conn.query("CREATE REL TABLE r3t(FROM Person TO Tag)").unwrap();

    for i in 0..151 {
        conn.query(&format!("CREATE (p:Person {{id: {i}}})")).unwrap();
    }
    for i in 0..101 {
        conn.query(&format!("CREATE (t:Tag {{id: {i}}})")).unwrap();
    }

    // Bulk WHERE-comparison CREATE (P48.3 pushdown keeps this fast).
    // Node predicates (`{id: {a}}`) are ignored in CREATE (BUG-A), so the
    // range form `a.id >= {a} AND a.id <= {a}` is used to pin the center.
    for a in 0..100 {
        conn.query(&format!(
            "MATCH (a:Person), (b:Person) WHERE a.id >= {a} AND a.id <= {a} AND b.id > a.id AND b.id <= a.id + 10 CREATE (a)-[:r1]->(b)"
        ))
        .unwrap();
    }
    conn.query(
        "MATCH (a:Person), (t:Tag) WHERE a.id >= 0 AND a.id <= 99 AND t.id >= 0 AND t.id <= 9 CREATE (a)-[:r2]->(t)",
    )
    .unwrap();
    conn.query(
        "MATCH (b:Person), (t:Tag) WHERE b.id >= 0 AND b.id <= 150 AND t.id >= 0 AND t.id <= 9 CREATE (b)-[:r3t]->(t)",
    )
    .unwrap();
    database
}

/// P48.4 triangle DB — N=41 Persons, three rel tables. For every triple
/// `a < b < c` the edges `a->b` (r1), `a->c` (r2), `b->c` (r3) exist, so the
/// triangle query returns exactly `C(41,3) = 10,660` rows.
fn setup_wcoj_triangle_db(dir: &TempDir) -> Arc<Database> {
    let db_path = dir.path().join("wcoj_triangle");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query("CREATE NODE TABLE Person(id INT64, PRIMARY KEY(id))").unwrap();
    conn.query("CREATE REL TABLE r1(FROM Person TO Person)").unwrap();
    conn.query("CREATE REL TABLE r2(FROM Person TO Person)").unwrap();
    conn.query("CREATE REL TABLE r3(FROM Person TO Person)").unwrap();

    for i in 0..41 {
        conn.query(&format!("CREATE (p:Person {{id: {i}}})")).unwrap();
    }

    // 6 bulk single-edge CREATE — `a` in [0,20] / [21,40] per rel table,
    // edges forward (`dst.id > src.id`) only.
    for (rel, a_lo, a_hi) in [
        ("r1", 0, 20),
        ("r1", 21, 40),
        ("r2", 0, 20),
        ("r2", 21, 40),
        ("r3", 0, 20),
        ("r3", 21, 40),
    ] {
        conn.query(&format!(
            "MATCH (a:Person), (b:Person) WHERE a.id >= {a_lo} AND a.id <= {a_hi} AND b.id > a.id CREATE (a)-[:{rel}]->(b)"
        ))
        .unwrap();
    }
    database
}

fn get_wcoj_fan_db() -> Connection {
    let dir = BENCH_DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let db = BENCH_DB_WCOJ_FAN.get_or_init(|| setup_wcoj_fan_db(dir));
    Connection::new(db)
}

fn get_wcoj_triangle_db() -> Connection {
    let dir = BENCH_DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let db = BENCH_DB_WCOJ_TRIANGLE.get_or_init(|| setup_wcoj_triangle_db(dir));
    Connection::new(db)
}

fn bench_simple_scan(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/simple");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("scan_all", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.id, p.name"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_filter(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/filter");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("filter_age_gt_50", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) WHERE p.age > 50 RETURN p.id, p.name"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("filter_active_and_young", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) WHERE p.active = true AND p.age < 30 RETURN p.id",
            ));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_aggregate(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/aggregate");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("count_all", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN COUNT(p)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("sum_age", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN SUM(p.age)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("avg_score", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN AVG(p.score)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("count_filter", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("min_max", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN MIN(p.age), MAX(p.age)"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_group_by(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/group_by");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("group_by_age", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.age, COUNT(p)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("group_by_active", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.active, COUNT(p), AVG(p.score)"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_sort(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/sort");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("sort_single_key", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.id, p.name ORDER BY p.age"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("sort_multi_key", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) RETURN p.id, p.name, p.age ORDER BY p.age, p.score",
            ));
            black_box(r.unwrap());
        })
    });
    group.bench_function("sort_limit_top10", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) RETURN p.id, p.name ORDER BY p.score DESC LIMIT 10",
            ));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_join(c: &mut Criterion) {
    let conn = get_full_db();
    let mut group = c.benchmark_group("ladybug/join");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("knows_join", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (a:Person)-[:Knows]->(b:Person) RETURN a.id, b.id LIMIT 1000",
            ));
            black_box(r.unwrap());
        })
    });
    group.bench_function("livesin_join", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person)-[:LivesIn]->(c:City) RETURN p.name, c.name LIMIT 1000",
            ));
            black_box(r.unwrap());
        })
    });
    group.bench_function("multi_hop", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (a:Person)-[:Knows]->(b:Person)-[:LivesIn]->(c:City) RETURN a.id, c.name LIMIT 500",
            ));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_complex(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/complex");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("filter_sort_limit", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) WHERE p.age > 25 RETURN p.id, p.name, p.score ORDER BY p.score DESC LIMIT 20",
            ));
            black_box(r.unwrap());
        })
    });
    group.bench_function("filter_agg", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) WHERE p.active = true RETURN COUNT(p), AVG(p.age), SUM(p.score)",
            ));
            black_box(r.unwrap());
        })
    });
    group.bench_function("order_limit_execute_only", |b| {
        let prepared = conn
            .prepare("MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)")
            .unwrap();
        b.iter(|| {
            let r = conn.execute(&prepared, vec![]);
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_buffer_manager(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/storage");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("primary_key_lookup", |b| {
        b.iter(|| {
            for i in 0..100 {
                let r = conn.query(black_box(&format!("MATCH (p:Person {{id: {i}}}) RETURN p.name")));
                black_box(r.unwrap());
            }
        })
    });
    group.bench_function("full_table_scan_100x", |b| {
        b.iter(|| {
            for _ in 0..100 {
                let r = conn.query(black_box("MATCH (p:Person) WHERE p.id < 100 RETURN p.name, p.age"));
                black_box(r.unwrap());
            }
        })
    });
    group.finish();
}

fn bench_100k_scan(c: &mut Criterion) {
    let conn = get_100k_db();
    let mut group = c.benchmark_group("ladybug_100k/scan");
    group.throughput(criterion::Throughput::Elements(100_000));
    group.bench_function("scan_all", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.id, p.name"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_100k_filter(c: &mut Criterion) {
    let conn = get_100k_db();
    let mut group = c.benchmark_group("ladybug_100k/filter");
    group.throughput(criterion::Throughput::Elements(100_000));
    group.bench_function("filter_age_gt_50", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) WHERE p.age > 50 RETURN p.id, p.name"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("filter_active_and_young", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) WHERE p.active = true AND p.age < 30 RETURN p.id",
            ));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_100k_sort(c: &mut Criterion) {
    let conn = get_100k_db();
    let mut group = c.benchmark_group("ladybug_100k/sort");
    group.throughput(criterion::Throughput::Elements(100_000));
    group.bench_function("sort_single_key", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.id, p.name ORDER BY p.age"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_100k_group_by(c: &mut Criterion) {
    let conn = get_100k_db();
    let mut group = c.benchmark_group("ladybug_100k/group_by");
    group.throughput(criterion::Throughput::Elements(100_000));
    group.bench_function("group_by_age", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.age, COUNT(p)"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_100k_aggregate(c: &mut Criterion) {
    let conn = get_100k_db();
    let mut group = c.benchmark_group("ladybug_100k/aggregate");
    group.throughput(criterion::Throughput::Elements(100_000));
    group.bench_function("count_all", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN COUNT(p)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("sum_age", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN SUM(p.age)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("filter_count", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) WHERE p.age > 50 RETURN COUNT(p)"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_1m_scan(c: &mut Criterion) {
    let conn = get_1m_db();
    let mut group = c.benchmark_group("ladybug_1m/scan");
    group.throughput(criterion::Throughput::Elements(1_000_000));
    group.bench_function("scan_all", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.id, p.name"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_1m_aggregate(c: &mut Criterion) {
    let conn = get_1m_db();
    let mut group = c.benchmark_group("ladybug_1m/aggregate");
    group.throughput(criterion::Throughput::Elements(1_000_000));
    group.bench_function("count_all", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN COUNT(p)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("sum_age", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN SUM(p.age)"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("filter_count", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) WHERE p.age > 50 RETURN COUNT(p)"));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_plan_cache(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/plan_cache");
    group.throughput(criterion::Throughput::Elements(10_000));

    // Warm up: first query in each benchmark populates the plan cache; all
    // subsequent iterations skip parse/bind/plan/optimize (cache hits).
    group.bench_function("repeated_query_cache_hit", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) WHERE p.age > 50 RETURN COUNT(p)"));
            black_box(r.unwrap());
        })
    });

    // Vary a constant each iteration → cache miss every time (full pipeline).
    // Represents the pre-P44.5 cost of repeated-but-different queries.
    let counter = std::sync::atomic::AtomicUsize::new(0);
    group.bench_function("varying_query_cache_miss", |b| {
        b.iter(|| {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 1000;
            let q = format!("MATCH (p:Person) WHERE p.age > {n} RETURN COUNT(p)");
            let r = conn.query(black_box(q.as_str()));
            black_box(r.unwrap());
        })
    });

    group.finish();
}

fn bench_wcoj(c: &mut Criterion) {
    let conn = get_wcoj_fan_db();
    let mut group = c.benchmark_group("ladybug/wcoj");
    group.throughput(criterion::Throughput::Elements(10_000));

    // Correctness first (P46.5 assert was wrong: expected 10k rows from a
    // 10k x 10k cross-product setup). Small design: star & chain both 10k rows.
    let star_sql = "MATCH (a:Person)-[:r1]->(b:Person), (a:Person)-[:r2]->(t:Tag) RETURN a.id, b.id, t.id";
    let star_res = conn.query(star_sql).unwrap();
    assert!(star_res.is_success(), "star query failed: {:?}", star_res.error_message);
    assert_eq!(
        star_res.num_rows, 10_000,
        "WCOJ star: expected 10,000 rows, got {}",
        star_res.num_rows
    );

    let chain_sql = "MATCH (a:Person)-[:r1]->(b:Person), (b:Person)-[:r3t]->(t:Tag) RETURN a.id, b.id, t.id";
    let chain_res = conn.query(chain_sql).unwrap();
    assert!(chain_res.is_success(), "chain query failed: {:?}", chain_res.error_message);
    assert_eq!(
        chain_res.num_rows, 10_000,
        "HashJoin chain: expected 10,000 rows, got {}",
        chain_res.num_rows
    );

    group.bench_function("star_intersect", |b| {
        b.iter(|| {
            let r = conn.query(black_box(star_sql));
            black_box(r.unwrap());
        })
    });
    group.bench_function("chain_hashjoin", |b| {
        b.iter(|| {
            let r = conn.query(black_box(chain_sql));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

fn bench_wcoj_triangle(c: &mut Criterion) {
    let conn = get_wcoj_triangle_db();
    let mut group = c.benchmark_group("ladybug/wcoj_triangle");
    group.throughput(criterion::Throughput::Elements(10_660));

    let triangle_sql = "MATCH (a:Person)-[:r1]->(b:Person), (a:Person)-[:r2]->(c:Person), (b:Person)-[:r3]->(c:Person) RETURN a.id, b.id, c.id";
    let res = conn.query(triangle_sql).unwrap();
    assert!(res.is_success(), "triangle query failed: {:?}", res.error_message);
    assert_eq!(
        res.num_rows, 10_660,
        "WCOJ triangle: expected C(41,3) = 10,660 rows, got {}",
        res.num_rows
    );

    group.bench_function("triangle_intersect", |b| {
        b.iter(|| {
            let r = conn.query(black_box(triangle_sql));
            black_box(r.unwrap());
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_simple_scan,
    bench_filter,
    bench_aggregate,
    bench_group_by,
    bench_sort,
    bench_join,
    bench_complex,
    bench_buffer_manager,
    bench_100k_scan,
    bench_100k_filter,
    bench_100k_aggregate,
    bench_100k_sort,
    bench_100k_group_by,
    bench_1m_scan,
    bench_1m_aggregate,
    bench_plan_cache,
    bench_wcoj,
    bench_wcoj_triangle,
);
criterion_main!(benches);
