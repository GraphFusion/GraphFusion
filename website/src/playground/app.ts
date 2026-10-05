import cytoscape, { type Core, type ElementDefinition } from "cytoscape";
import { EditorView, keymap } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { basicSetup } from "codemirror";
import { StreamLanguage } from "@codemirror/language";
import { indentWithTab } from "@codemirror/commands";
import EngineWorker from "./engine.worker?worker";
import { examples } from "./examples";
import type {
  Cell,
  GraphNode,
  GraphProjection,
  Statement,
  Response,
  Reply,
  Request,
} from "./types";

const $ = <T extends HTMLElement = HTMLElement>(id: string) =>
  document.getElementById(id) as T;
const runButton = $<HTMLButtonElement>("run");
const picker = $<HTMLSelectElement>("element-picker");
const emptyGraph: GraphProjection = {
  nodes: [],
  edges: [],
  truncated: false,
  unresolved: 0,
};
let graph = emptyGraph;
let cy: Core;
let worker: Worker;
let serial = 0;
let busy = true;
let ready = false;
let response: Response | undefined;
let statement: Statement | undefined;
let elapsed = 0;
let lastQuery = "";
let history: string[] = [];
const pending = new Map<
  number,
  { resolve: (reply: Reply) => void; reject: (error: Error) => void }
>();
const colors = [
  "#79a9d1",
  "#7cbba7",
  "#c5a2d8",
  "#d7b780",
  "#d58f9d",
  "#85babe",
];
const palette = new Map<string, string>([
  ["Person", colors[0]],
  ["Project", colors[1]],
  ["Organization", colors[2]],
]);
function color(label: string) {
  if (!palette.has(label))
    palette.set(label, colors[palette.size % colors.length]);
  return palette.get(label)!;
}
const gql = StreamLanguage.define({
  token(stream) {
    if (stream.eatSpace()) return null;
    if (stream.match("//")) {
      stream.skipToEnd();
      return "comment";
    }
    if (
      stream.match(/'(?:[^'\\]|\\.|'')*'/) ||
      stream.match(/"(?:[^"\\]|\\.)*"/)
    )
      return "string";
    if (stream.match(/\b\d+(?:\.\d+)?\b/)) return "number";
    if (
      stream.match(
        /\b(?:MATCH|RETURN|WHERE|LIMIT|ORDER|BY|DESC|ASC|INSERT|SET|DELETE|DETACH|REMOVE|CREATE|GRAPH|ANY|SESSION|START|TRANSACTION|COMMIT|ROLLBACK|FINISH|AS|AND|OR|NOT|IS|OPTIONAL|LET|NEXT|USE|ALL|SHORTEST|DISTINCT|GROUP)\b/i,
      )
    )
      return "keyword";
    if (stream.match(/\b(?:true|false|null)\b/i)) return "atom";
    if (stream.match(/[A-Za-z_][\w]*/)) return "variableName";
    stream.next();
    return null;
  },
});
const editor = new EditorView({
  state: EditorState.create({
    doc: examples[0].query,
    extensions: [
      basicSetup,
      gql,
      keymap.of([
        {
          key: "Mod-Enter",
          run: () => {
            void runQuery();
            return true;
          },
        },
        {
          key: "Shift-Enter",
          run: () => {
            void runQuery();
            return true;
          },
        },
        indentWithTab,
      ]),
      EditorView.contentAttributes.of({
        "aria-label": "GQL query",
        spellcheck: "false",
      }),
      EditorView.lineWrapping,
    ],
  }),
  parent: $("editor"),
});
function setQuery(query: string, title = "Custom query") {
  editor.dispatch({
    changes: { from: 0, to: editor.state.doc.length, insert: query },
  });
  $("query-title").textContent = title;
  document
    .querySelectorAll("[data-example]")
    .forEach((button) =>
      button.classList.toggle(
        "active",
        examples[Number((button as HTMLElement).dataset.example)]?.query ===
          query,
      ),
    );
}
function setBusy(value: boolean) {
  busy = value;
  runButton.disabled = value || !ready;
  runButton.innerHTML = value
    ? '<span aria-hidden="true">◌</span> Running…'
    : '<span aria-hidden="true">▶</span> Run query';
  $("stop").hidden = !value;
  document
    .querySelector(".result-card")!
    .setAttribute("aria-busy", String(value));
}
function status(text: string, loading = false) {
  const element = $("engine-status");
  element.replaceChildren();
  const dot = document.createElement("span");
  dot.className = `status-dot${loading ? " loading" : ""}`;
  element.append(dot, document.createTextNode(text));
}
function send(action: Request["action"], query?: string): Promise<Reply> {
  const id = ++serial;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    worker.postMessage({ id, action, query } satisfies Request);
  });
}
function destroyWorker() {
  worker?.terminate();
  for (const request of pending.values())
    request.reject(new Error("Session reset."));
  pending.clear();
}
async function initialize() {
  destroyWorker();
  ready = false;
  setBusy(true);
  status("Loading engine…", true);
  const current = new EngineWorker();
  worker = current;
  current.onmessage = (event: MessageEvent<Reply>) => {
    if (worker !== current) return;
    if (event.data.progress) {
      status(event.data.progress, true);
      return;
    }
    const request = pending.get(event.data.id);
    if (!request) return;
    pending.delete(event.data.id);
    request.resolve(event.data);
  };
  current.onerror = (event) => {
    for (const request of pending.values())
      request.reject(new Error(event.message || "The worker could not start."));
    pending.clear();
    ready = false;
    status("Engine unavailable");
  };
  response = undefined;
  statement = undefined;
  lastQuery = "";
  renderStatement(undefined);
  $("error").hidden = true;
  $("empty-title").textContent = "Connecting the dots";
  $("empty-description").textContent =
    "Loading GraphFusion and your sample graph…";
  try {
    const reply = await send("init");
    if (worker !== current) return;
    if (reply.error) throw new Error(reply.error);
    ready = true;
    status("WASM engine ready");
    setBusy(false);
    setQuery(examples[0].query, examples[0].title);
    await runQuery();
  } catch (error) {
    if (worker !== current) return;
    showError(
      `Could not load the browser engine. ${error instanceof Error ? error.message : String(error)}\nUse “Reset sample data” to retry.`,
    );
    $("empty-title").textContent = "Engine unavailable";
    $("empty-description").textContent =
      "Check the error above, then reset the session to retry.";
    status("Engine unavailable");
    setBusy(false);
  }
}
function showError(message: string) {
  $("error").textContent = message;
  $("error").hidden = false;
}
async function runQuery() {
  if (busy || !ready) return;
  const query = editor.state.doc.toString().trim();
  if (!query) {
    showError("Write a GQL query first, or choose an example.");
    editor.focus();
    return;
  }
  const current = worker;
  setBusy(true);
  status("Executing locally…", true);
  $("error").hidden = true;
  try {
    const reply = await send("run", query);
    if (worker !== current) return;
    if (reply.error) {
      if (reply.fatal) {
        ready = false;
        status("Session stopped — reset to continue");
      } else status("WASM engine ready");
      throw new Error(reply.error);
    }
    response = reply.result!;
    elapsed = reply.elapsed || 0;
    lastQuery = query;
    const selector = $<HTMLSelectElement>("statement-picker");
    selector.replaceChildren();
    response.statements.forEach((item, i) => {
      const option = document.createElement("option");
      option.value = String(i);
      option.textContent = `Statement ${i + 1} · ${item.kind}`;
      selector.append(option);
    });
    selector.hidden = response.statements.length < 2;
    selector.value = String(Math.max(0, response.statements.length - 1));
    renderStatement(response.statements.at(-1));
    remember(query);
    status("WASM engine ready");
  } catch (error) {
    if (worker !== current) return;
    showError(
      `${error instanceof Error ? error.message : String(error)}${response ? "\nThe result below is from the last successful query." : ""}`,
    );
  } finally {
    if (worker === current) setBusy(false);
  }
}
async function resetSession() {
  const current = worker;
  setBusy(true);
  status("Resetting sample data…", true);
  $("error").hidden = true;
  try {
    const reply = await send("reset");
    if (worker !== current) return;
    if (reply.error) throw new Error(reply.error);
    response = undefined;
    statement = undefined;
    lastQuery = "";
    setQuery(examples[0].query, examples[0].title);
    setBusy(false);
    await runQuery();
  } catch (error) {
    if (worker !== current) return;
    ready = false;
    showError(
      `Could not reset the session. ${error instanceof Error ? error.message : String(error)}`,
    );
    status("Session stopped — reset to continue");
    setBusy(false);
  }
}
function text(value: Cell): string {
  if (value === null) return "null";
  if (typeof value === "object") {
    if (
      !Array.isArray(value) &&
      typeof value.element === "string" &&
      typeof value.id === "string"
    ) {
      const element = [...graph.nodes, ...graph.edges].find(
        (n) => n.id === value.id,
      );
      return element ? caption(element) : value.id;
    }
    return JSON.stringify(value);
  }
  return String(value);
}
function caption(element: GraphNode): string {
  for (const name of ["name", "title", "label"]) {
    const value = element.properties[name];
    if (typeof value === "string" || typeof value === "number")
      return String(value);
  }
  return element.labels.join(" · ") || element.id;
}
function make<T extends keyof HTMLElementTagNameMap>(
  tag: T,
  content?: string,
  className?: string,
) {
  const element = document.createElement(tag);
  if (content !== undefined) element.textContent = content;
  if (className) element.className = className;
  return element;
}
function renderStatement(item?: Statement) {
  statement = item;
  graph = item?.graph || emptyGraph;
  $("export").toggleAttribute("disabled", !item);
  const transactionOpen = response?.transaction !== "Idle";
  const state = item?.pending
    ? transactionOpen
      ? " · transaction pending"
      : " · transaction result"
    : "";
  $("result-meta").textContent = item
    ? `${item.kind === "query" ? `${item.rowCount} rows` : "Command complete"} · ${elapsed < 1 ? "<1" : Math.round(elapsed)} ms${state}`
    : "Preparing your workspace…";
  $("graph-counts").textContent =
    `${graph.nodes.length} nodes · ${graph.edges.length} relationships${item?.affected ? ` · ${item.affected} affected` : ""}`;
  const notices: string[] = [];
  if (item?.rowCount && item.rowCount > 1000)
    notices.push(
      "Showing the first 1,000 rows. Add LIMIT to explore a smaller result.",
    );
  if (graph.truncated)
    notices.push(
      "The graph shows at most 500 returned elements plus relationship endpoints.",
    );
  if (graph.unresolved)
    notices.push(
      `${graph.unresolved} returned references are absent from this snapshot (for example, deleted elements).`,
    );
  if (item?.pending)
    notices.push(
      transactionOpen
        ? "Changes are provisional. Run COMMIT to keep them, or ROLLBACK to undo them."
        : "This result comes from a transaction that has since closed. Run a new query to see the current graph.",
    );
  const banner = $("result-banner");
  banner.textContent = notices.join(" ");
  banner.hidden = !notices.length;
  $("logical-plan").textContent =
    item?.logicalPlan || "No query plan for this statement.";
  $("physical-plan").textContent =
    item?.physicalPlan || "No query plan for this statement.";
  renderGraph();
  renderTable(item);
  inspect("");
  if (graph.nodes.length || graph.edges.length) changeView("graph");
  if (item && !graph.nodes.length && !graph.edges.length) {
    $("empty-title").textContent =
      item.kind === "command" || item.affected
        ? "Statement complete"
        : item.rowCount
          ? "A table of answers"
          : "No matches yet";
    $("empty-description").textContent =
      item.kind === "command" || item.affected
        ? "Run a MATCH query to explore the updated graph."
        : item.rowCount
          ? "This query returns values. Open Table to see them, or RETURN a node, relationship or path to visualize it."
          : "Try a different pattern or choose an example to explore the data.";
    if (item.rowCount) changeView("table");
  }
}
function renderGraph() {
  $("search").setAttribute("value", "");
  ($("search") as HTMLInputElement).value = "";
  const definitions: ElementDefinition[] = graph.nodes.map((node, i) => ({
    position: {
      x: 180 * Math.cos((i * 2 * Math.PI) / graph.nodes.length),
      y: 180 * Math.sin((i * 2 * Math.PI) / graph.nodes.length),
    },
    data: {
      id: node.id,
      caption: caption(node),
      color: color(node.labels[0] || "Node"),
    },
  }));
  const nodeIds = new Set(graph.nodes.map((node) => node.id));
  for (const edge of graph.edges)
    if (nodeIds.has(edge.source) && nodeIds.has(edge.target)) {
      definitions.push({
        data: {
          id: edge.id,
          source: edge.source,
          target: edge.target,
          caption: edge.labels.join(" · ") || "related",
          directed: edge.directed ? "triangle" : "none",
        },
      });
    }
  cy.batch(() => {
    cy.elements().remove();
    cy.add(definitions);
  });
  if (graph.nodes.length > 150)
    cy.layout({ name: "grid", fit: true, padding: 68 }).run();
  else
    cy.layout({
      name: "cose",
      animate: false,
      fit: true,
      padding: 68,
      nodeRepulsion: () => 7000,
      idealEdgeLength: () => 100,
      edgeElasticity: () => 90,
      gravity: 0.3,
      numIter: 400,
      randomize: false,
      componentSpacing: 80,
    }).run();
  $("graph-empty").hidden = !!definitions.length;
  const legend = $("legend");
  legend.replaceChildren();
  const counts = new Map<string, number>();
  graph.nodes.forEach((n) => {
    for (const label of n.labels.length ? n.labels : ["Node"])
      counts.set(label, (counts.get(label) || 0) + 1);
  });
  for (const [label, count] of counts) {
    const pill = make("span", undefined, "legend-pill");
    const dot = make("span", undefined, "legend-dot");
    dot.style.setProperty("--node-color", color(label));
    pill.append(
      dot,
      document.createTextNode(label),
      make("span", String(count), "legend-count"),
    );
    legend.append(pill);
  }
  picker.replaceChildren(new Option("Select a node or relationship", ""));
  const nodes = document.createElement("optgroup");
  nodes.label = "Nodes";
  graph.nodes.forEach((n) =>
    nodes.append(new Option(`${caption(n)} · ${n.labels.join(", ")}`, n.id)),
  );
  const edges = document.createElement("optgroup");
  edges.label = "Relationships";
  graph.edges.forEach((e) =>
    edges.append(
      new Option(
        `${captionById(e.source)} → ${captionById(e.target)} · ${e.labels.join(", ")}`,
        e.id,
      ),
    ),
  );
  picker.append(nodes, edges);
  picker.disabled = !definitions.length;
}
function captionById(id: string) {
  const node = graph.nodes.find((n) => n.id === id);
  return node ? caption(node) : id;
}
function inspect(id: string) {
  const container = $("inspector-content");
  container.replaceChildren();
  picker.value = id;
  cy.elements().unselect();
  const node = graph.nodes.find((n) => n.id === id);
  const edge = graph.edges.find((e) => e.id === id);
  const element = node || edge;
  $("selected-kind").textContent = edge
    ? "Relationship"
    : node
      ? "Node"
      : "Element";
  if (!element) {
    const hint = make("div", undefined, "inspector-empty");
    hint.append(
      make("span", "↖"),
      make("strong", "A closer look"),
      make("p", "Select an element to explore its labels and properties."),
    );
    container.append(hint);
    return;
  }
  cy.getElementById(id).select();
  container.append(make("h2", caption(element), "element-caption"));
  const labels = make("div", undefined, "element-labels");
  element.labels.forEach((label) =>
    labels.append(make("span", label, "element-label")),
  );
  container.append(labels);
  if (edge) {
    container.append(
      make(
        "div",
        edge.directed ? "Directed relationship" : "Undirected relationship",
        "property-heading",
      ),
    );
    const endpoints = make("div");
    for (const [name, endpoint] of [
      ["From", edge.source],
      ["To", edge.target],
    ]) {
      const button = make(
        "button",
        `${name}: ${captionById(endpoint)}`,
        "endpoint-button",
      );
      button.addEventListener("click", () => inspect(endpoint));
      endpoints.append(button, make("br"));
    }
    container.append(endpoints);
  }
  const properties = Object.entries(element.properties);
  const heading = make("div", undefined, "property-heading");
  heading.append(
    make("span", "Properties"),
    make("span", String(properties.length)),
  );
  container.append(heading);
  const list = make("dl", undefined, "property-list");
  for (const [key, value] of properties) {
    const row = make("div", undefined, "property-row");
    row.append(make("dt", key), make("dd", text(value)));
    list.append(row);
  }
  if (!properties.length) list.append(make("p", "No properties", "muted"));
  container.append(list, make("div", element.id, "identity"));
}
function renderCell(container: HTMLElement, value: Cell) {
  if (value === null) {
    container.append(make("span", "null", "null-cell"));
    return;
  }
  if (
    typeof value === "object" &&
    !Array.isArray(value) &&
    typeof value.id === "string" &&
    typeof value.element === "string"
  ) {
    const button = make("button", text(value), "ref-link");
    const id = value.id;
    button.title = id;
    button.addEventListener("click", () => {
      changeView("graph");
      inspect(id);
    });
    container.append(button);
    return;
  }
  if (Array.isArray(value)) {
    container.append(document.createTextNode("["));
    value.forEach((cell, i) => {
      if (i) container.append(document.createTextNode(", "));
      renderCell(container, cell);
    });
    container.append(document.createTextNode("]"));
    return;
  }
  if (typeof value === "object" && Array.isArray(value.path)) {
    renderCell(container, value.path);
    return;
  }
  container.append(document.createTextNode(text(value)));
}
function renderTable(item?: Statement) {
  const container = $("table-container");
  container.replaceChildren();
  if (!item?.columns?.length || !item.rows?.length) {
    container.append(
      make(
        "p",
        item?.affected
          ? `Statement complete. ${item.affected} elements affected.`
          : "This statement returned no rows.",
        "empty-table",
      ),
    );
    return;
  }
  const table = make("table");
  const thead = make("thead");
  const tr = make("tr");
  item.columns.forEach((column) => {
    const th = make("th", column);
    th.scope = "col";
    tr.append(th);
  });
  thead.append(tr);
  table.append(thead);
  const body = make("tbody");
  for (const row of item.rows) {
    const tr = make("tr");
    row.forEach((value) => {
      const td = make("td");
      renderCell(td, value);
      tr.append(td);
    });
    body.append(tr);
  }
  table.append(body);
  container.append(table);
}
function changeView(view: string) {
  for (const name of ["graph", "table", "plan"]) {
    const tab = $(`tab-${name}`);
    tab.setAttribute("aria-selected", String(name === view));
    tab.tabIndex = name === view ? 0 : -1;
    $(`view-${name}`).hidden = name !== view;
  }
  if (view === "graph")
    requestAnimationFrame(() => {
      cy.resize();
      cy.fit(undefined, 65);
    });
}
function remember(query: string) {
  history = [query, ...history.filter((old) => old !== query)].slice(0, 10);
  try {
    localStorage.setItem("graphfusion-query-history", JSON.stringify(history));
  } catch {
    /* Private mode can disallow local storage. */
  }
  renderHistory();
}
function renderHistory() {
  $("history-count").textContent = String(history.length);
  const container = $("history");
  container.replaceChildren();
  history.forEach((query) => {
    const button = make("button", query.replace(/\s+/g, " "), "history-item");
    button.title = query;
    button.addEventListener("click", () => {
      setQuery(query, "Recent query");
      editor.focus();
    });
    container.append(button);
  });
  if (!history.length)
    container.append(
      make("p", "Your queries will appear here.", "muted sidebar-empty"),
    );
}

cy = cytoscape({
  container: $("graph"),
  elements: [],
  layout: { name: "preset" },
  minZoom: 0.15,
  maxZoom: 3,
  selectionType: "single",
  boxSelectionEnabled: false,
  style: [
    {
      selector: "node",
      style: {
        "background-color": "data(color)",
        width: 46,
        height: 46,
        "border-width": 3,
        "border-color": "#ffffff",
        label: "data(caption)",
        "font-size": 11,
        "font-family": "system-ui, sans-serif",
        "font-weight": 500,
        color: "#53697f",
        "text-valign": "bottom",
        "text-margin-y": 10,
        "text-wrap": "ellipsis",
        "text-max-width": "105px",
        "text-background-color": "#fcfdfe",
        "text-background-opacity": 0.9,
        "text-background-padding": "2px",
        "overlay-padding": 6,
      },
    },
    {
      selector: "edge",
      style: {
        width: 1.3,
        "line-color": "#bdcbd7",
        "target-arrow-color": "#aebfce",
        "target-arrow-shape": (edge) =>
          edge.data("directed") === "triangle" ? "triangle" : "none",
        "curve-style": "bezier",
        "arrow-scale": 0.65,
        label: "data(caption)",
        "font-size": 8,
        "font-family": "system-ui, sans-serif",
        color: "#91a1b2",
        "text-rotation": "autorotate",
        "text-background-color": "#fcfdfe",
        "text-background-opacity": 1,
        "text-background-padding": "3px",
        "text-margin-y": -5,
      },
    },
    {
      selector: "node:selected",
      style: {
        "border-color": "#196f79",
        "border-width": 3,
        "overlay-opacity": 0.08,
        "overlay-color": "#197780",
      },
    },
    {
      selector: "edge:selected",
      style: {
        "line-color": "#197780",
        "target-arrow-color": "#197780",
        width: 2.5,
        color: "#197780",
      },
    },
    { selector: ".dim", style: { opacity: 0.14 } },
    {
      selector: ".found",
      style: { "border-color": "#196f79", "border-width": 3 },
    },
  ],
});
cy.on("tap", "node, edge", (event) => inspect(event.target.id()));
cy.on("tap", (event) => {
  if (event.target === cy) inspect("");
});
new ResizeObserver(() => cy.resize()).observe($("graph"));
runButton.addEventListener("click", () => void runQuery());
$("clear-query").addEventListener("click", () => {
  setQuery("");
  editor.focus();
});
document.querySelectorAll("[data-example]").forEach((button) =>
  button.addEventListener("click", () => {
    const example = examples[Number((button as HTMLElement).dataset.example)];
    setQuery(example.query, example.title);
    if (innerWidth <= 800)
      ($("workspace-options") as HTMLDetailsElement).open = false;
    void runQuery();
  }),
);
document.querySelectorAll("[data-view]").forEach((button) => {
  button.addEventListener("click", () =>
    changeView((button as HTMLElement).dataset.view!),
  );
  button.addEventListener("keydown", (event) => {
    const keyboard = event as KeyboardEvent;
    const views = ["graph", "table", "plan"];
    const index = views.indexOf((button as HTMLElement).dataset.view!);
    if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(keyboard.key)) {
      keyboard.preventDefault();
      const next =
        keyboard.key === "Home"
          ? 0
          : keyboard.key === "End"
            ? 2
            : (index + (keyboard.key === "ArrowRight" ? 1 : 2)) % 3;
      changeView(views[next]);
      $(`tab-${views[next]}`).focus();
    }
  });
});
picker.addEventListener("change", () => inspect(picker.value));
$("statement-picker").addEventListener("change", () =>
  renderStatement(
    response?.statements[
      Number(($("statement-picker") as HTMLSelectElement).value)
    ],
  ),
);
$("zoom-in").addEventListener("click", () =>
  cy.zoom({
    level: Math.min(3, cy.zoom() * 1.25),
    renderedPosition: { x: cy.width() / 2, y: cy.height() / 2 },
  }),
);
$("zoom-out").addEventListener("click", () =>
  cy.zoom({
    level: Math.max(0.15, cy.zoom() / 1.25),
    renderedPosition: { x: cy.width() / 2, y: cy.height() / 2 },
  }),
);
$("fit").addEventListener("click", () => cy.fit(undefined, 65));
$("layout").addEventListener("click", () =>
  cy
    .layout({
      name: "circle",
      padding: 65,
      animate: !matchMedia("(prefers-reduced-motion: reduce)").matches,
      animationDuration: 350,
    })
    .run(),
);
$("search").addEventListener("input", () => {
  const term = ($("search") as HTMLInputElement).value.toLowerCase().trim();
  cy.elements().removeClass("dim found");
  if (!term) return;
  const ids = new Set(
    graph.nodes
      .filter((n) =>
        `${caption(n)} ${n.labels.join(" ")} ${JSON.stringify(n.properties)}`
          .toLowerCase()
          .includes(term),
      )
      .map((n) => n.id),
  );
  cy.nodes().forEach((node) => {
    node.addClass(ids.has(node.id()) ? "found" : "dim");
  });
  cy.edges().forEach((edge) => {
    if (!ids.has(edge.source().id()) && !ids.has(edge.target().id()))
      edge.addClass("dim");
  });
});
$("export").addEventListener("click", () => {
  if (!statement) return;
  const url = URL.createObjectURL(
    new Blob(
      [
        JSON.stringify(
          { query: lastQuery, statement, elapsedMs: elapsed },
          null,
          2,
        ),
      ],
      { type: "application/json" },
    ),
  );
  const a = make("a");
  a.href = url;
  a.download = "graphfusion-result.json";
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
});
const dialog = $("reset-dialog") as HTMLDialogElement;
$("reset").addEventListener("click", () => {
  if (!ready) void initialize();
  else {
    dialog.returnValue = "cancel";
    dialog.showModal();
  }
});
dialog.addEventListener("close", () => {
  if (dialog.returnValue === "reset") {
    changeView("graph");
    void resetSession();
  }
});
$("stop").addEventListener("click", () => {
  changeView("graph");
  void initialize();
});
if (innerWidth <= 800)
  ($("workspace-options") as HTMLDetailsElement).open = false;
try {
  const saved: unknown = JSON.parse(
    localStorage.getItem("graphfusion-query-history") || "[]",
  );
  if (Array.isArray(saved))
    history = saved
      .filter((q): q is string => typeof q === "string" && q.length <= 100_000)
      .slice(0, 10);
} catch {
  /* History is optional. */
}
renderHistory();
void initialize();
