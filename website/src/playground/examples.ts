export const examples = [
  {
    title: "Explore the graph",
    description: "People, projects & connections",
    query: "MATCH (a)-[r]->(b)\nRETURN a, r, b\nLIMIT 100",
  },
  {
    title: "Find Alice’s neighbors",
    description: "Follow a relationship",
    query: "MATCH (a:Person {name: 'Alice'})-[r]->(b)\nRETURN a, r, b",
  },
  {
    title: "Discover a path",
    description: "Two hops through the team",
    query: "MATCH p = (a:Person {name: 'Alice'})-[:Knows]->{2}(b)\nRETURN p",
  },
  {
    title: "Team insights",
    description: "Group and count connections",
    query:
      "MATCH (p:Person)-[:Builds]->(project:Project)\nRETURN project.name AS project, count(p) AS contributors\nORDER BY contributors DESC",
  },
  {
    title: "Update a property",
    description: "Change data, see the result",
    query:
      "MATCH (p:Person {name: 'Alice'})\nSET p.role = 'Graph architect'\nRETURN p",
  },
];
export const seed = `CREATE GRAPH playground ANY GRAPH;
SESSION SET GRAPH playground;
INSERT
 (alice:Person {name: 'Alice', role: 'Database engineer', city: 'Shanghai'}),
 (bob:Person {name: 'Bob', role: 'Systems engineer', city: 'Berlin'}),
 (maya:Person {name: 'Maya', role: 'Data scientist', city: 'London'}),
 (noah:Person {name: 'Noah', role: 'Rust developer', city: 'Tokyo'}),
 (lea:Person {name: 'Lea', role: 'Frontend engineer', city: 'Paris'}),
 (owen:Person {name: 'Owen', role: 'Platform engineer', city: 'New York'}),
 (gf:Project {name: 'GraphFusion', language: 'Rust', focus: 'Graph queries'}),
 (df:Project {name: 'DataFusion', language: 'Rust', focus: 'Query execution'}),
 (arrow:Project {name: 'Arrow', language: 'Rust', focus: 'Columnar data'}),
 (lab:Organization {name: 'Graph Lab', kind: 'Research'}),
 (community:Organization {name: 'Open Source', kind: 'Community'}),
 (alice)-[:Knows {since: 2022}]->(bob),
 (alice)-[:Knows {since: 2023}]->(maya),
 (bob)-[:Knows {since: 2021}]->(noah),
 (maya)-[:Knows {since: 2024}]->(lea),
 (noah)-[:Knows {since: 2023}]->(owen),
 (alice)-[:Builds]->(gf), (bob)-[:Builds]->(gf), (lea)-[:Builds]->(gf),
 (noah)-[:Builds]->(df), (owen)-[:Builds]->(df), (maya)-[:Builds]->(arrow),
 (gf)-[:Uses]->(df), (df)-[:Uses]->(arrow),
 (alice)-[:WorksAt]->(lab), (maya)-[:WorksAt]->(lab),
 (bob)-[:ContributesTo]->(community), (owen)-[:ContributesTo]->(community);`;
