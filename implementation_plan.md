# Lambda Evaluation Infrastructure Implementation Plan

Based on your request, we will build the full Lambda evaluation infrastructure for list predicates (`ANY`, `ALL`, `NONE`, `SINGLE`), moving away from the "truthy check" workaround. 

## Context & Research Findings
The Kuzu Rust port currently parses `ListPredicate` correctly (Layers 1-3 are complete). However, the evaluator currently evaluates the `predicate` against a `mini_chunk` that ONLY contains the list item, making it impossible to reference outer variables. Furthermore, variable name resolution in the evaluator relies heavily on string parsing of indices or falling back to the first field.

To properly evaluate lambda predicates like `ANY(x IN list WHERE x.prop > y)`, we need to:
1. Ensure the `predicate` is evaluated in a context that contains *both* the list item (`x`) and all outer variables (`y`).
2. Map the lambda variable (`x`) to the correct index in this execution context so the evaluator can resolve it.
3. Optimize the evaluation by flattening the list elements across all rows into a single large chunk, evaluating the predicate once, and aggregating the results (similar to `ListLambdaEvaluator` in C++), rather than doing an inefficient N*M nested loop of single-row evaluations.

## Proposed Changes

### 1. AST Variable Rewriting for Lambda Parameters
**Component**: `kuzu-processor/src/expression_evaluator.rs`
- We will write a helper function `rewrite_variable(expr: &mut Expression, old_name: &str, new_name: &str)` to traverse the `predicate` AST and replace the lambda `var_name` with its positional index (e.g., `"1"` if it's appended as the second field). This matches the existing `parse::<usize>()` lookup in `evaluate_variable`.

### 2. Flattened List Evaluation (ListLambdaEvaluator)
**Component**: `kuzu-processor/src/expression_evaluator.rs`
- Rewrite `evaluate_list_predicate` to avoid the nested row-by-row 1-element chunk execution.
- **Algorithm**:
  1. Evaluate the `list` expression to get a `ValueVector` of lists.
  2. Compute total number of elements across all lists in all rows.
  3. Create a flattened `DataChunk` containing:
     - All fields from the original chunk, duplicated/expanded for each list element (to preserve outer variable context).
     - A new field at the end containing the flattened list elements.
  4. Rewrite the lambda `var_name` in the `predicate` AST to the index of this new field (`chunk.fields.len()`).
  5. Call `self.evaluate(predicate, &flattened_chunk)` **exactly once**.
  6. Iterate through the boolean results and aggregate them back into the original row indices using the `Quantifier` logic (`Any`, `All`, `None`, `Single`).

### 3. Binder Scope (Optional / Future-proofing)
**Component**: `kuzu-binder/src/binder.rs`
- Since the executor uses index-based variable resolution based on the chunk structure, the binder's primary job for list predicates is type checking. We will update `resolve_expression` for `ListPredicate` to extract the inner type of the list, push it to a cloned `variables` scope, resolve the `predicate` type, and ensure it returns `LogicalTypeID::Bool`.

## Verification Plan
### Automated Tests
- I will run `cargo check` and `cargo test --workspace` to ensure nothing breaks.
- I will add a new test in `kuzu-processor` (or modify the existing ones if applicable) to specifically test `ANY(x IN [1,2,3] WHERE x > 1)` and verify it handles both inner and outer variables correctly.

## User Review Required
Please review this implementation plan. The flattening approach (Step 2) is exactly how the C++ Kuzu engine handles it (`ListLambdaEvaluator`) and provides significant performance benefits over the naive nested loop. If you approve, click **Proceed** and I will begin execution.
