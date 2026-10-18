use akar_main::connection::Connection;
use akar_main::database::{Database, SystemConfig};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let filter = args.get(1).map(|s| s.as_str()).unwrap_or("all");

    println!("LadybugDB Benchmark Runner");
    println!("==========================");
    println!();

    let db_dir = std::env::temp_dir().join("ladybug_bench_db");
    let _ = std::fs::remove_dir_all(&db_dir);
    let config = SystemConfig::default();
    let database = Arc::new(Database::new(&db_dir, config).unwrap());
    let conn = Connection::new(&database);

    setup_schema(&conn);
    load_data(&conn);
    println!("Database loaded: 10k Person nodes, 10 Cities, 5k Knows edges, 10k LivesIn edges");
    println!();

    let mut total = std::time::Duration::ZERO;
    let mut count = 0u32;

    macro_rules! run_bench {
        ($name:expr, $query:expr) => {
            if filter == "all" || filter.contains($name) {
                let iterations = 200;
                let start = Instant::now();
                for _ in 0..iterations {
                    let r = conn.query($query).unwrap();
                    let _ = r;
                }
                let elapsed = start.elapsed();
                let per_iter = elapsed / iterations;
                println!("  {:<40} {:>10.2?}", $name, per_iter);
                total += per_iter;
                count += 1;
            }
        };
    }

    if filter == "all" || filter.contains("simple") {
        println!("--- Simple Queries ---");
        run_bench!("scan_all", "MATCH (p:Person) RETURN p.ID, p.name");
        run_bench!("filter_age", "MATCH (p:Person) WHERE p.age > 50 RETURN p.ID");
        run_bench!("filter_active", "MATCH (p:Person) WHERE p.active = true RETURN p.ID");
        println!();
    }

    if filter == "all" || filter.contains("agg") {
        println!("--- Aggregation ---");
        run_bench!("count_all", "MATCH (p:Person) RETURN COUNT(p)");
        run_bench!("sum_age", "MATCH (p:Person) RETURN SUM(p.age)");
        run_bench!("avg_score", "MATCH (p:Person) RETURN AVG(p.score)");
        run_bench!("count_filtered", "MATCH (p:Person) WHERE p.age > 30 RETURN COUNT(p)");
        run_bench!("min_max", "MATCH (p:Person) RETURN MIN(p.age), MAX(p.age)");
        println!();
    }

    if filter == "all" || filter.contains("group") {
        println!("--- GROUP BY ---");
        run_bench!("group_by_age", "MATCH (p:Person) RETURN p.age, COUNT(p)");
        run_bench!(
            "group_by_active",
            "MATCH (p:Person) RETURN p.active, COUNT(p), AVG(p.score)"
        );
        println!();
    }

    if filter == "all" || filter.contains("sort") {
        println!("--- Sort ---");
        run_bench!("sort_single", "MATCH (p:Person) RETURN p.ID, p.name ORDER BY p.age");
        run_bench!(
            "sort_multi",
            "MATCH (p:Person) RETURN p.ID, p.name ORDER BY p.age, p.score"
        );
        run_bench!(
            "sort_top10",
            "MATCH (p:Person) RETURN p.ID ORDER BY p.score DESC LIMIT 10"
        );
        println!();
    }

    if filter == "all" || filter.contains("join") {
        println!("--- Joins ---");
        run_bench!(
            "knows_join",
            "MATCH (a:Person)-[:Knows]->(b:Person) RETURN a.ID, b.ID LIMIT 1000"
        );
        run_bench!(
            "city_join",
            "MATCH (p:Person)-[:LivesIn]->(c:City) RETURN p.name, c.name LIMIT 1000"
        );
        run_bench!(
            "multi_hop",
            "MATCH (a:Person)-[:Knows]->(b:Person)-[:LivesIn]->(c:City) RETURN a.ID, c.name LIMIT 500"
        );
        println!();
    }

    if filter == "all" || filter.contains("complex") {
        println!("--- Complex Queries ---");
        run_bench!(
            "filter_sort_limit",
            "MATCH (p:Person) WHERE p.age > 25 RETURN p.ID, p.name ORDER BY p.score DESC LIMIT 20"
        );
        run_bench!(
            "filter_agg_complex",
            "MATCH (p:Person) WHERE p.active = true RETURN COUNT(p), AVG(p.age), SUM(p.score)"
        );
        println!();
    }

    println!("==========================");
    println!("  Total benchmarks: {}", count);
    println!(
        "  Average per query: {:.2?}",
        if count > 0 {
            total / count
        } else {
            std::time::Duration::ZERO
        }
    );
    println!();
    println!("Usage: ladybug [filter]");
    println!("  filter: all (default), simple, agg, group, sort, join, complex");

    let _ = std::fs::remove_dir_all(&db_dir);
}

fn setup_schema(conn: &Connection) {
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
}

fn load_data(conn: &Connection) {
    let csv_dir = std::env::temp_dir();
    let csv_path = csv_dir.join("ladybug_person_10k.csv");
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

    let _ = std::fs::remove_file(&csv_path);
}
