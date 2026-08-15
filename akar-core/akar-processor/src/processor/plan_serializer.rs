use akar_planner::logical_operator::LogicalOperator;

#[allow(unreachable_patterns)]
pub fn serialize_plan_tree(op: &LogicalOperator, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let prefix = if depth > 0 { "├─ " } else { "" };

    let op_name = match op {
        LogicalOperator::ScanNode(s) => format!("ScanNode({})", s.table_name),
        LogicalOperator::ScanRel(s) => format!("ScanRel({})", s.table_name),
        LogicalOperator::Filter(_) => "Filter".to_string(),
        LogicalOperator::Projection(p) => format!("Projection({} cols)", p.expressions.len()),
        LogicalOperator::HashJoin(hj) => format!("HashJoin({} keys)", hj.join_keys.len()),
        LogicalOperator::CrossProduct(_) => "CrossProduct".to_string(),
        LogicalOperator::OrderBy(ob) => format!("OrderBy({} keys)", ob.sort_keys.len()),
        LogicalOperator::TopK(tk) => format!(
            "TopK(limit={}, offset={}, {} keys)",
            tk.limit,
            tk.offset,
            tk.sort_keys.len()
        ),
        LogicalOperator::Limit(l) => format!("Limit({})", l.limit),
        LogicalOperator::Aggregate(a) => {
            format!("Aggregate({} aggs, {} group_by)", a.aggregates.len(), a.group_by.len())
        }
        LogicalOperator::Union(u) => format!("Union({})", if u.all { "ALL" } else { "DISTINCT" }),
        LogicalOperator::Flatten(_) => "Flatten".to_string(),
        LogicalOperator::TableFunctionCall(tf) => format!("TableFunctionCall({})", tf.function_name),
        LogicalOperator::CopyFrom(cf) => format!("CopyFrom({})", cf.table_name),
        LogicalOperator::BatchInsert(bi) => format!("BatchInsert({}, {} rows)", bi.table_name, bi.rows.len()),
        LogicalOperator::IndexLookup(il) => format!("IndexLookup({})", il.table_name),
        LogicalOperator::Delete(dl) => format!("Delete({})", dl.table_name),
        LogicalOperator::Set(sl) => format!(
            "Set({}.{})",
            sl.table_name,
            sl.items.first().map(|i| i.column_name.as_str()).unwrap_or("?")
        ),
        LogicalOperator::OptionalMatch(_) => "OptionalMatch".to_string(),
        LogicalOperator::OptionalExtend(oe) => {
            format!("OptionalExtend({} via {})", oe.rel_var, oe.rel_table_name)
        }
        LogicalOperator::Unwind(uw) => format!("Unwind({})", uw.variable),
        LogicalOperator::Foreach(fe) => format!("Foreach({})", fe.variable),
        LogicalOperator::Merge(m) => format!("Merge({})", m.table_name),
        LogicalOperator::MergeRel(mr) => format!("MergeRel({})", mr.rel_table_name),
        LogicalOperator::SemiJoin(_) => "SemiJoin".to_string(),
        LogicalOperator::AntiJoin(_) => "AntiJoin".to_string(),
        LogicalOperator::VectorSimilarityScan(vs) => format!("VectorSimilarityScan(k={})", vs.top_k),
        LogicalOperator::ArtIndexRangeScan(ars) => format!("ArtIndexRangeScan({})", ars.table_name),
        LogicalOperator::Explain(_) => "Explain".to_string(),
        LogicalOperator::Intersect(_) => "Intersect".to_string(),
        LogicalOperator::RecursiveExtend(re) => {
            format!("RecursiveExtend({}..{})", re.lower_bound, re.upper_bound)
        }
        LogicalOperator::Accumulate(ac) => format!("Accumulate({:?})", ac.accumulate_type),
        LogicalOperator::ExpressionsScan(es) => format!("ExpressionsScan({} vars)", es.expressions.len()),
        LogicalOperator::CountRelTable(crt) => format!("CountRelTable({})", crt.table_name),
        LogicalOperator::CreateNodeTable(ct) => format!("CreateNodeTable({})", ct.name),
        LogicalOperator::CreateRelTable(ct) => format!("CreateRelTable({})", ct.name),
        LogicalOperator::DropTable(dt) => format!("DropTable({})", dt.name),
        LogicalOperator::AlterTable(at) => format!("AlterTable({})", at.table_name),
        LogicalOperator::CreateIndex(ci) => format!("CreateIndex({})", ci.index_name),
        LogicalOperator::DropIndex(di) => format!("DropIndex({})", di.index_name),
        LogicalOperator::CreateVectorIndex(vi) => format!("CreateVectorIndex({})", vi.index_name),
        LogicalOperator::CreateSequence(cs) => format!("CreateSequence({})", cs.name),
        LogicalOperator::DropSequence(ds) => format!("DropSequence({})", ds.name),
        LogicalOperator::CreateDml(cd) => format!("CreateDml({})", cd.table_name),
        LogicalOperator::CreateNode(cn) => format!("CreateNode({})", cn.table_name),
        LogicalOperator::CreateRel(cr) => format!("CreateRel({})", cr.table_name),
        LogicalOperator::Extend(ex) => format!(
            "Extend({}->{} via {})",
            ex.bound_node_var, ex.dst_node_var, ex.rel_table_name
        ),
        LogicalOperator::ExportDatabase(ed) => format!("ExportDatabase({})", ed.file_path),
        LogicalOperator::ImportDatabase(id) => format!("ImportDatabase({})", id.file_path),
        LogicalOperator::CreateFtsIndex(c) => format!("CreateFtsIndex({})", c.index_name),
        LogicalOperator::FtsScan(s) => format!("FtsScan({})", s.index_name),
        _ => format!("{:?}", op),
    };

    let card_str = format!("[cardinality={}]", op.cardinality());
    let mut result = format!("{indent}{prefix}{op_name} {card_str}\n");

    let children = op.children();
    for (i, child) in children.iter().enumerate() {
        let child_str = serialize_plan_tree(child, depth + 1);
        if i == children.len() - 1 {
            let adjusted = child_str.replacen("├─ ", "└─ ", 1);
            result.push_str(&adjusted);
        } else {
            result.push_str(&child_str);
        }
    }

    result
}
