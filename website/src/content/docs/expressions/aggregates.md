---
title: "Aggregate functions"
description: "Summarize graph and relational result rows."
sidebar:
  order: 15
---

Supported aggregates are COUNT, SUM, AVG, MIN, MAX, COLLECT_LIST, STDDEV_POP, STDDEV_SAMP, PERCENTILE_CONT and PERCENTILE_DISC. ALL and DISTINCT select whether duplicate input values contribute.

```gql test
MATCH (p:Person)
RETURN COUNT(*) AS people, AVG(p.age) AS average_age,
       MIN(p.age) AS youngest, MAX(p.age) AS oldest;
```

COUNT(*) counts rows. COUNT(value) and COUNT(element) omit nulls. All aggregates omit null inputs. An empty global group produces COUNT = 0, COLLECT_LIST = [] and null numeric results. COLLECT_LIST order is unspecified.

Integer SUM accumulates in a wider representation before a checked Int64 conversion. Non-finite floating results are errors. Use [GROUP BY and HAVING](/GraphFusion/query/group-by/) to form and filter groups.

```gql test
MATCH (p:Person)
RETURN PERCENTILE_CONT(p.age, 0.5) AS median,
       PERCENTILE_DISC(p.age, 0.5) AS discrete_median;
```

The fraction must be a row-independent numeric expression in [0, 1]; a null fraction returns null. Continuous percentiles return Float64. Discrete percentiles preserve the input numeric type, including the exact selected integer.

## Function reference

- [COUNT](/GraphFusion/expressions/count/)
- [SUM and AVG](/GraphFusion/expressions/sum-avg/)
- [MIN and MAX](/GraphFusion/expressions/min-max/)
- [COLLECT_LIST](/GraphFusion/expressions/collect-list/)
- [Standard deviation](/GraphFusion/expressions/standard-deviation/)
- [Percentiles](/GraphFusion/expressions/percentiles/)
