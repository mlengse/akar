use akar_main::connection::Connection;
use akar_main::database::{Database, SystemConfig};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;

static BENCH_DIR: OnceLock<TempDir> = OnceLock::new();
static BENCH_DB_NODES: OnceLock<Arc<Database>> = OnceLock::new();
static BENCH_DB_FULL: OnceLock<Arc<Database>> = OnceLock::new();

fn setup_nodes_db(dir: &TempDir) -> Arc<Database> {
    let db_path = dir.path().join("nodes_only");
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(db_path, config).unwrap());
    let conn = Connection::new(&database);

    conn.query(
        "CREATE NODE TABLE Person(ID INT64, name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (ID))",
    )
    .unwrap();
    conn.query("CREATE NODE TABLE City(name STRING, population INT64, country STRING, PRIMARY KEY (name))")
        .unwrap();

    let csv_path = dir.path().join("person_10k.csv");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&csv_path).unwrap();
        writeln!(f, "ID,name,age,score,active").unwrap();
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
        "CREATE NODE TABLE Person(ID INT64, name STRING, age INT64, score DOUBLE, active BOOL, PRIMARY KEY (ID))",
    )
    .unwrap();
    conn.query("CREATE NODE TABLE City(name STRING, population INT64, country STRING, PRIMARY KEY (name))")
        .unwrap();
    conn.query("CREATE REL TABLE Knows(FROM Person TO Person, since INT64)")
        .unwrap();
    conn.query("CREATE REL TABLE LivesIn(FROM Person TO City, since INT64)")
        .unwrap();

    let csv_path = dir.path().join("person_10k.csv");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&csv_path).unwrap();
        writeln!(f, "ID,name,age,score,active").unwrap();
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

    for i in 0..5000u64 {
        let target = (i + 1) % 10_000;
        conn.query(&format!(
            "MATCH (a:Person {{ID: {i}}}), (b:Person {{ID: {target}}}) CREATE (a)-[:Knows {{since: 2020}}]->(b)"
        ))
        .unwrap();
    }
    for i in 0..10_000u64 {
        let city_idx = i % 10;
        let city = cities[city_idx as usize];
        conn.query(&format!(
            "MATCH (p:Person {{ID: {i}}}), (c:City {{name: '{city}'}}) CREATE (p)-[:LivesIn {{since: 2015}}]->(c)"
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

fn bench_simple_scan(c: &mut Criterion) {
    let conn = get_nodes_db();
    let mut group = c.benchmark_group("ladybug/simple");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.bench_function("scan_all", |b| {
        b.iter(|| {
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.ID, p.name"));
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
            let r = conn.query(black_box("MATCH (p:Person) WHERE p.age > 50 RETURN p.ID, p.name"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("filter_active_and_young", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) WHERE p.active = true AND p.age < 30 RETURN p.ID",
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
            let r = conn.query(black_box("MATCH (p:Person) RETURN p.ID, p.name ORDER BY p.age"));
            black_box(r.unwrap());
        })
    });
    group.bench_function("sort_multi_key", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) RETURN p.ID, p.name, p.age ORDER BY p.age, p.score",
            ));
            black_box(r.unwrap());
        })
    });
    group.bench_function("sort_limit_top10", |b| {
        b.iter(|| {
            let r = conn.query(black_box(
                "MATCH (p:Person) RETURN p.ID, p.name ORDER BY p.score DESC LIMIT 10",
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
                "MATCH (a:Person)-[:Knows]->(b:Person) RETURN a.ID, b.ID LIMIT 1000",
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
                "MATCH (a:Person)-[:Knows]->(b:Person)-[:LivesIn]->(c:City) RETURN a.ID, c.name LIMIT 500",
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
                "MATCH (p:Person) WHERE p.age > 25 RETURN p.ID, p.name, p.score ORDER BY p.score DESC LIMIT 20",
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
                let r = conn.query(black_box(&format!("MATCH (p:Person {{ID: {i}}}) RETURN p.name")));
                black_box(r.unwrap());
            }
        })
    });
    group.bench_function("full_table_scan_100x", |b| {
        b.iter(|| {
            for _ in 0..100 {
                let r = conn.query(black_box("MATCH (p:Person) WHERE p.ID < 100 RETURN p.name, p.age"));
                black_box(r.unwrap());
            }
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
);
criterion_main!(benches);
