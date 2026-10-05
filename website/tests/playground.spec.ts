import { test, expect, type Page } from "@playwright/test";
test.setTimeout(120_000);

async function open(page: Page) {
  await page.goto("/playground/");
  await page.waitForFunction(
    () =>
      !document.getElementById("error")!.hidden ||
      document
        .getElementById("graph-counts")!
        .textContent!.includes("11 nodes · 17 relationships"),
    undefined,
    { timeout: 90_000 },
  );
  const alert = page.locator("#error");
  await expect(
    alert,
    (await alert.textContent()) || "The browser engine should initialize.",
  ).toBeHidden();
  await expect(page.locator("#graph-counts")).toContainText(
    "11 nodes · 17 relationships",
  );
  await expect(
    page.getByRole("button", { name: "Run query", exact: true }),
  ).toBeEnabled();
  await expect(page.getByRole("alert")).toBeHidden();
}
async function run(page: Page, query: string) {
  await page.getByRole("textbox", { name: "GQL query" }).fill(query);
  await page.getByRole("button", { name: "Run query", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Run query", exact: true }),
  ).toBeEnabled();
  await expect(page.getByRole("alert")).toBeHidden();
}
async function example(page: Page, index: number) {
  const summary = page.locator("#workspace-options summary");
  if (
    (await summary.isVisible()) &&
    (await page.locator("#workspace-options").getAttribute("open")) === null
  )
    await summary.click();
  await page.locator(`[data-example="${index}"]`).click();
  await expect(
    page.getByRole("button", { name: "Run query", exact: true }),
  ).toBeEnabled();
  await expect(page.getByRole("alert")).toBeHidden();
}

test("real browser engine visualizes elements, paths, properties and query plans", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await open(page);
  await page
    .getByLabel("Inspect an element")
    .selectOption({ label: "Alice · Person" });
  await expect(page.locator("#inspector-content")).toContainText(
    "Database engineer",
  );
  await page.getByLabel("Find a node in the result").fill("Alice");
  await page.getByRole("button", { name: "Fit graph", exact: true }).click();
  await example(page, 2);
  await expect(page.locator("#graph-counts")).toContainText(
    "5 nodes · 4 relationships",
  );
  await page.getByRole("tab", { name: "Table", exact: false }).click();
  await expect(page.locator("tbody tr")).toHaveCount(2);
  await expect(page.locator("tbody")).toContainText("Alice");
  await page.getByRole("tab", { name: "Plan", exact: false }).click();
  await expect(page.locator("#physical-plan")).not.toContainText(
    "No query plan",
  );
  await example(page, 3);
  await expect(
    page.getByRole("tab", { name: "Table", exact: false }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("tbody")).toContainText("GraphFusion");
  expect(errors).toEqual([]);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("queries and mutations work with the browser completely offline", async ({
  page,
  context,
}) => {
  await open(page);
  await context.setOffline(true);
  await run(
    page,
    "INSERT (p:Person {name:'Offline Ada', role:'Local explorer'}) RETURN p",
  );
  await expect(page.locator("#graph-counts")).toContainText(
    "1 nodes · 0 relationships",
  );
  await page
    .getByLabel("Inspect an element")
    .selectOption({ label: "Offline Ada · Person" });
  await expect(page.locator("#inspector-content")).toContainText(
    "Local explorer",
  );
  await run(
    page,
    "MATCH (p:Person {name:'Offline Ada'}) SET p.role='Updated locally' RETURN p",
  );
  await page
    .getByLabel("Inspect an element")
    .selectOption({ label: "Offline Ada · Person" });
  await expect(page.locator("#inspector-content")).toContainText(
    "Updated locally",
  );
  await run(page, "MATCH (p:Person {name:'Offline Ada'}) DETACH DELETE p");
  await run(page, "MATCH (p:Person {name:'Offline Ada'}) RETURN p");
  await expect(page.locator("#result-meta")).toContainText("0 rows");
  await run(page, "RETURN 9223372036854775807 AS exact_integer");
  await expect(page.locator("tbody")).toContainText("9223372036854775807");
  await run(
    page,
    "START TRANSACTION; INSERT (:Person {name: 'Provisional'}); MATCH (p {name: 'Provisional'}) RETURN p",
  );
  await expect(page.locator("#result-banner")).toContainText("provisional");
  await run(page, "ROLLBACK; MATCH (p {name:'Provisional'}) RETURN p");
  await expect(page.locator("#result-meta")).toContainText("0 rows");
});

test("errors are recoverable, exports reflect the query, and reset restores sample data", async ({
  page,
  context,
}) => {
  await open(page);
  await page.getByRole("textbox", { name: "GQL query" }).fill("MATCH (");
  await page.getByRole("button", { name: "Run query", exact: true }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Run query", exact: true }),
  ).toBeEnabled();
  await run(page, "MATCH (p:Person {name:'Alice'}) RETURN p");
  const downloaded = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export JSON", exact: false }).click();
  const download = await downloaded;
  expect(download.suggestedFilename()).toBe("graphfusion-result.json");
  const stream = await download.createReadStream();
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(chunk);
  const exported = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  expect(exported.query).toBe("MATCH (p:Person {name:'Alice'}) RETURN p");
  expect(exported.statement.graph.nodes).toHaveLength(1);
  expect(exported.statement.graph.nodes[0].properties.name).toBe("Alice");
  await context.setOffline(true);
  await run(page, "MATCH (p) DETACH DELETE p");
  const summary = page.locator("#workspace-options summary");
  if (
    (await summary.isVisible()) &&
    (await page.locator("#workspace-options").getAttribute("open")) === null
  )
    await summary.click();
  await page
    .getByRole("button", { name: "Reset sample data", exact: false })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Reset sample data", exact: true })
    .click();
  await expect(page.locator("#graph-counts")).toContainText(
    "11 nodes · 17 relationships",
    { timeout: 90_000 },
  );
  await expect(page.getByRole("alert")).toBeHidden();
  await run(page, "INSERT (:Person {name:'Keep me'})");
  await page
    .getByRole("button", { name: "Reset sample data", exact: false })
    .click();
  await page.getByRole("dialog").press("Escape");
  await run(page, "MATCH (p {name:'Keep me'}) RETURN p");
  await expect(page.locator("#result-meta")).toContainText("1 rows");
});
