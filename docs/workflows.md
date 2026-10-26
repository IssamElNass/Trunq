# Query, result and diagnostic workflows

[Documentation](README.md) · [User guide](usage.md) · [Troubleshooting](troubleshooting.md)

## A disposable practice table

Run this whole script in an **Auto-commit** query tab. The table exists only in that tab's database session and disappears when the session closes:

```sql
CREATE TEMP TABLE trunq_docs_orders (
    id integer PRIMARY KEY,
    status text NOT NULL,
    total numeric NOT NULL CHECK (total >= 0)
);
INSERT INTO trunq_docs_orders VALUES
    (1, 'pending', 25.00),
    (2, 'paid', 80.00),
    (3, 'paid', 45.50);
SELECT * FROM trunq_docs_orders ORDER BY id;
```

Keep the same tab for the examples below. The object browser and another query tab have separate sessions and cannot use this temporary table. The larger [sample database](usage.md#try-it-with-the-sample-database) is available for browsing tables and relationships.

## Parameters, templates and reusable queries

Use values, not SQL fragments, in named parameters:

```sql
SELECT id, status, total
FROM trunq_docs_orders
WHERE total >= :minimum::numeric
  AND status = :status::text
ORDER BY id;
```

Enter `40` and `paid` to return two rows. Supply values without SQL quotes: entering `'paid'` would send the quote characters as part of the value. Empty input sends empty text. Input whose trimmed text is `NULL`, ignoring case, sends SQL NULL. To query the literal word `NULL`, write that literal in the SQL instead.

Repeated `:name` placeholders in one statement reuse a bound value. Add an explicit cast when PostgreSQL cannot infer the type, for example `SELECT :value::text IS NULL`. Placeholders inside quoted strings and comments are not prompts, and parameters cannot stand in for identifiers or an ORDER BY clause. For a list of IDs, use separate parameters or a typed PostgreSQL array rather than expecting one text value to expand into an IN list.

Table context-menu templates can contain `?` as a visual marker. Replace it before execution; use `:name` if you want a prompt. Select **Save as snippet** from the query toolbar to keep reusable SQL. Opening a snippet through the command palette can prompt for its named values again.

History records executed SQL and summaries, not a reusable set of parameter values. A parameterized history entry can therefore contain `$1`, `$2`, and so on. Keep the original `:name` query as a snippet or `.sql` file when you want to run it again with prompts.

The editor runs PostgreSQL SQL. `psql` meta commands such as `\d` and `\copy` are not executed. Use the object browser for structure and the import dialog for CSV `COPY FROM STDIN`.

## Auto-commit, Manual and failed transactions

| State or control | What it means | What to do next |
|---|---|---|
| Auto-commit, no open transaction | Normal statements finish their transaction when execution/result fetching finishes | Run the next statement normally |
| Manual | Trunq starts a transaction before running SQL when needed | Choose Commit or Rollback when the work is finished |
| In transaction | A transaction is open; it may also be the implicit transaction for a paged result | Finish fetching, close the result's session, or explicitly finish a transaction |
| Transaction failed | A statement failed inside an open transaction | Roll back before continuing normal SQL |

Changing the Auto-commit/Manual selector does not itself commit or roll back an already-open transaction. Transactions belong to one tab, and closing that tab or losing its connection rolls back uncommitted work.

To practice a manual transaction, select **Manual**, run this statement and inspect its RETURNING result:

```sql
UPDATE trunq_docs_orders
SET status = 'paid'
WHERE id = 1
RETURNING *;
```

Choose **Rollback**, then run `SELECT * FROM trunq_docs_orders WHERE id = 1;` to confirm that `pending` is restored. That SELECT can open another manual transaction; finish it with Commit or Rollback too. In a failed transaction, repeated retries will not repair the state: first roll back, then correct the SQL.

The table data editor saves a batch atomically. Within an existing explicit transaction, it uses a savepoint and leaves the outer transaction open. **Dry run** also uses a transaction or savepoint but rolls its change back before reporting completion. A dry-run row preview is bounded by the fetch page, and database sequences or external side effects of server functions are not generally undone by transaction rollback. Plain **EXPLAIN** plans a query; **EXPLAIN ANALYZE** executes it.

## Understand which data a tool uses

| Tool | Data scope |
|---|---|
| Grid search, local sort, local filter chips and aggregates | Rows loaded into that result; pending grid edits can affect its local view |
| A table view's WHERE / ORDER BY bar | A new SQL query on the server |
| Pinned result | Loaded rows retained in memory when another query runs |
| Chart | The loaded result set, independently of local grid search and filter chips |
| Visible-row file export | Loaded rows matching the grid view, in its displayed row order |
| Loaded-row file export | All rows already fetched, in their result order |
| Entire-result export | SQL executed again on a separate connection |

For a large SELECT in a multi-statement script, run that SELECT on its own when you need to keep fetching pages. Only the final statement in a run keeps a fetchable cursor. Running another query closes the previous cursor, even when its loaded rows are pinned. Fetch what you need before replacing the run.

Pinning does not create a durable snapshot: pinned rows disappear when the app closes. A server export can see newer committed data, and its separate connection cannot see the tab's temporary tables, session-local settings or uncommitted changes. Export **Loaded rows** for the temporary practice table in this guide.

File export scopes choose rows, not columns. File exports include all result columns, including columns hidden in the grid. Select the desired columns in your SQL if you need a narrower file. Save pending table edits and refresh the result before exporting their saved values; pending edits are not written into the underlying result buffer by the file exporter.

CSV distinguishes SQL NULL from empty text. JSON and TypeScript consumers can lose numeric precision when they parse a large integer as a JavaScript number. To preserve its exact decimal representation as a JSON string, select it as text, for example `SELECT id::text AS id FROM trunq_docs_orders;`. Export formats and destination handling are described in [the user guide](usage.md#choosing-an-export).

## Compare records and inspect values

1. In the grid, select two to six rows and choose **Compare rows** from the context menu. Use **Differences only** to hide fields that agree.
2. Choose **View → Row card** for a vertical record layout. Use the arrow controls or left/right keys to move between loaded rows.
3. Open a JSON/JSONB cell to inspect its tree. Search the document and use a node's context menu to copy its value, JSON path or PostgreSQL accessor.
4. Hover a foreign-key cell for a related-record peek, or use the command modifier and click to follow it. A single-column foreign key and suitable source-column/catalog metadata are needed; arbitrary computed expressions do not always retain that metadata.

**View → Timestamps** changes the presentation of `timestamptz` values between Raw, Local and UTC. It does not change stored values or the database session's timezone. Treat grid display preferences separately from query results and file exports.

## Make a chart with meaningful axes

Run this aggregate query in the practice tab, then choose **Chart**:

```sql
SELECT status, count(*) AS orders, sum(total) AS revenue
FROM trunq_docs_orders
GROUP BY status
ORDER BY status;
```

Choose the category/time column for X and numeric columns for Y. **Auto** uses lines for a numeric or time axis and bars for categories; **Bars** and **Lines** override that choice. Trunq avoids ID columns and very different scales when choosing default series. If you select measures with very different scales, it warns you to plot them separately.

Charts do not perform a GROUP BY for you and do not inherit grid search/filter chips. Filter and aggregate in SQL before charting. Up to eight series are selectable; bar charts display at most 300 rows. Use an ORDER BY and a suitable LIMIT or aggregation to control the categories being plotted. **Load all** still consumes memory, even if a chart renders fewer categories.

## Investigate a slow or blocked query

1. Check the connection identity and the query/fetch/render timing (hover the result row count for the full breakdown). A slow server execution, a slow transfer and expensive rendering are different problems.
2. For a query waiting on a lock, inspect the **Lock detective** information and the blocker. A blocker can be idle in a transaction while still holding locks.
3. Open **Active sessions** to compare PIDs, application names, wait events, query age and transaction age. The view refreshes while visible; **Hide idle** can hide otherwise healthy client sessions.
4. **Cancel** stops the current statement. **Terminate** disconnects the whole backend and rolls back its open transaction. Both require PostgreSQL permission; do not assume access to another user's session.
5. Coordinate with the session owner before interrupting unrelated work. Cancelling a statement in an explicit transaction can leave that transaction failed until its owner rolls it back.

Some query text and statistics are unavailable to a limited role. Ask the database administrator which monitoring access is appropriate; a missing field is not proof that a session has no query or is healthy. Trunq does not automatically grant roles or install extensions.

## Use health findings and plans as evidence

The health tab generates suggested SQL for review; it does not apply fixes automatically. Table row counts in the browser are estimates until you request an exact count. Statistics can be old or have been reset, so an unused-index finding or a low cache ratio needs workload context before action.

For a plan, start with plain EXPLAIN. Use EXPLAIN ANALYZE only when running the query is acceptable. Inspect actual versus estimated rows and loop counts; row-estimate discrepancies often warrant checking statistics and query predicates before changing indexes.

**Slow queries (pg_stat_statements)** opens an editable query against that extension. Your administrator must preload and install the extension in the relevant database. Its statistics are aggregated since the applicable reset, rather than a list of individual executions.

For relationships, open **ER diagram** from a schema, database or table. Schema entry points limit the graph and table entry points focus its neighbourhood. The graph reflects catalogued foreign keys; application-only relationships do not appear automatically. See [the feature reference](features.md#explore-relationships-and-database-metadata) for screenshots.

## Review a production operation

Confirm the profile/database/user, use server-enforced least privilege and review the complete SQL before applying changes. The [production guide](production.md) covers verified TLS, pinned SSH and credential migration. For extensions use [Manage extensions](extensions.md); for schedules/auditing use [Database tools](administration.md). Local history is not an audit log.

![Monthly revenue chart from the sample database](screenshots/chart-revenue.png)

This chart aggregates `shop.orders` by month with `date_trunc` before plotting, using only sample data. The chart reflects its query result rather than grid filter chips. See [performance](performance.md#chart-preparation-cache-25-october-2026) for cache behavior and memory costs.
