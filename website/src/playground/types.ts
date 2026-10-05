export type Cell =
  | null
  | boolean
  | number
  | string
  | Cell[]
  | { [key: string]: Cell };
export interface GraphNode {
  id: string;
  labels: string[];
  properties: Record<string, Cell>;
}
export interface GraphEdge extends GraphNode {
  source: string;
  target: string;
  directed: boolean;
}
export interface GraphProjection {
  nodes: GraphNode[];
  edges: GraphEdge[];
  truncated: boolean;
  unresolved: number;
}
export interface Statement {
  kind: "query" | "command";
  columns?: string[];
  rows?: Cell[][];
  rowCount?: number;
  graph?: GraphProjection;
  logicalPlan?: string;
  physicalPlan?: string;
  affected: number;
  commit: string;
  pending: boolean;
  action?: string;
}
export interface Response {
  statements: Statement[];
  transaction: string;
}
export type Request = {
  id: number;
  action: "init" | "reset" | "run";
  query?: string;
};
export type Reply = {
  id: number;
  result?: Response;
  error?: string;
  fatal?: boolean;
  elapsed?: number;
  progress?: string;
};
