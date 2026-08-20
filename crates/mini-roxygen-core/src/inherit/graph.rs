//! Deterministic local-inheritance graph analysis.

use std::collections::{BTreeMap, BTreeSet};

/// The canonical cycle path for one cyclic strongly connected component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleReport<Node> {
    /// The path starts and ends at the lexicographically smallest node in the
    /// component.
    pub path: Vec<Node>,
}

/// One strongly connected component and its cycle status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StronglyConnectedComponent<Node> {
    /// The component's nodes in sorted order.
    pub nodes: Vec<Node>,
    /// The canonical cycle when this component is cyclic.
    pub cycle: Option<CycleReport<Node>>,
}

impl<Node> StronglyConnectedComponent<Node> {
    /// Returns whether this component contains a cycle.
    #[must_use]
    pub fn is_cyclic(&self) -> bool {
        self.cycle.is_some()
    }
}

/// The complete deterministic analysis of a local inheritance graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphAnalysis<Node> {
    /// Strongly connected components in deterministic component order.
    pub components: Vec<StronglyConnectedComponent<Node>>,
    /// One canonical report for each cyclic component.
    pub cycles: Vec<CycleReport<Node>>,
    /// Nodes ordered so dependencies precede their dependants. Nodes in one
    /// cyclic component are emitted in sorted order because their internal
    /// dependency order is not acyclic.
    pub dependency_order: Vec<Node>,
    /// Nodes belonging to cyclic components, for local-only recovery later.
    pub cyclic_nodes: BTreeSet<Node>,
}

/// Analyzes a graph whose edge list maps each node to its requested donors.
///
/// The map supplies node identity and the vector for each node preserves
/// request order. Nodes appearing only as edge targets are included. Node
/// traversal is sorted by `Ord`, while edge traversal remains request-ordered.
/// No diagnostics or recovery policy are chosen here.
pub fn analyze_inheritance_graph<Node>(edge_list: &BTreeMap<Node, Vec<Node>>) -> GraphAnalysis<Node>
where
    Node: Clone + Ord,
{
    let nodes = all_nodes(edge_list);
    let index_by_node = nodes
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, node)| (node, index))
        .collect::<BTreeMap<_, _>>();
    let edges = nodes
        .iter()
        .map(|node| {
            edge_list
                .get(node)
                .into_iter()
                .flatten()
                .map(|target| {
                    *index_by_node
                        .get(target)
                        .expect("all target nodes are indexed")
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut tarjan = Tarjan::new(&edges);
    for node in 0..nodes.len() {
        if tarjan.indices[node].is_none() {
            tarjan.visit(node);
        }
    }

    let mut components = tarjan
        .components
        .into_iter()
        .map(|component| {
            let mut component_nodes = component
                .into_iter()
                .map(|index| nodes[index].clone())
                .collect::<Vec<_>>();
            component_nodes.sort();
            let is_cyclic = component_nodes.len() > 1
                || component_nodes.first().is_some_and(|node| {
                    let index = index_by_node.get(node).expect("component node is indexed");
                    edges[*index].contains(index)
                });
            let cycle = is_cyclic.then(|| CycleReport {
                path: canonical_cycle(&component_nodes, edge_list),
            });
            StronglyConnectedComponent {
                nodes: component_nodes,
                cycle,
            }
        })
        .collect::<Vec<_>>();
    components.sort_by(|left, right| left.nodes[0].cmp(&right.nodes[0]));

    let mut cycles = Vec::new();
    let mut cyclic_nodes = BTreeSet::new();
    for component in &components {
        if let Some(cycle) = &component.cycle {
            cycles.push(cycle.clone());
            cyclic_nodes.extend(component.nodes.iter().cloned());
        }
    }

    let dependency_order = dependency_order(&nodes, &edges, &components);

    GraphAnalysis {
        components,
        cycles,
        dependency_order,
        cyclic_nodes,
    }
}

fn all_nodes<Node>(edge_list: &BTreeMap<Node, Vec<Node>>) -> Vec<Node>
where
    Node: Clone + Ord,
{
    let mut nodes = edge_list.keys().cloned().collect::<BTreeSet<_>>();
    nodes.extend(edge_list.values().flatten().cloned());
    nodes.into_iter().collect()
}

fn canonical_cycle<Node>(component: &[Node], edge_list: &BTreeMap<Node, Vec<Node>>) -> Vec<Node>
where
    Node: Clone + Ord,
{
    let start = component
        .first()
        .expect("a cyclic component is non-empty")
        .clone();
    let members = component.iter().cloned().collect::<BTreeSet<_>>();
    let mut path = vec![start.clone()];
    let mut visited = BTreeSet::from([start.clone()]);
    if find_cycle_path(&start, &start, &members, edge_list, &mut visited, &mut path) {
        return path;
    }
    unreachable!("a cyclic strongly connected component has a return path")
}

fn find_cycle_path<Node>(
    current: &Node,
    start: &Node,
    members: &BTreeSet<Node>,
    edge_list: &BTreeMap<Node, Vec<Node>>,
    visited: &mut BTreeSet<Node>,
    path: &mut Vec<Node>,
) -> bool
where
    Node: Clone + Ord,
{
    for next in edge_list.get(current).into_iter().flatten() {
        if !members.contains(next) {
            continue;
        }
        if next == start {
            path.push(start.clone());
            return true;
        }
        if visited.insert(next.clone()) {
            path.push(next.clone());
            if find_cycle_path(next, start, members, edge_list, visited, path) {
                return true;
            }
            path.pop();
            visited.remove(next);
        }
    }
    false
}

fn dependency_order<Node>(
    nodes: &[Node],
    edges: &[Vec<usize>],
    components: &[StronglyConnectedComponent<Node>],
) -> Vec<Node>
where
    Node: Clone + Ord,
{
    let component_id = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            (
                index,
                components
                    .iter()
                    .position(|component| component.nodes.binary_search(node).is_ok())
                    .expect("each node belongs to one component"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut condensed_edges = vec![BTreeSet::new(); components.len()];
    let mut indegree = vec![0usize; components.len()];
    for (source, targets) in edges.iter().enumerate() {
        for &target in targets {
            let source_component = component_id[&source];
            let target_component = component_id[&target];
            if source_component != target_component
                && condensed_edges[target_component].insert(source_component)
            {
                indegree[source_component] += 1;
            }
        }
    }

    // Condensed edges point from a donor component to its recipient, so a
    // zero-indegree component is ready to resolve first.
    let mut ready = BTreeSet::new();
    for (component, &degree) in indegree.iter().enumerate() {
        if degree == 0 {
            ready.insert(component);
        }
    }
    let mut order = Vec::with_capacity(components.len());
    while let Some(component) = ready.pop_first() {
        order.push(component);
        for &dependent in &condensed_edges[component] {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                ready.insert(dependent);
            }
        }
    }
    assert_eq!(
        order.len(),
        components.len(),
        "the condensation graph is acyclic"
    );

    order
        .into_iter()
        .flat_map(|component| components[component].nodes.iter().cloned())
        .collect()
}

struct Tarjan<'a> {
    edges: &'a [Vec<usize>],
    next_index: usize,
    indices: Vec<Option<usize>>,
    lowlinks: Vec<usize>,
    stack: Vec<usize>,
    on_stack: Vec<bool>,
    components: Vec<Vec<usize>>,
}

impl<'a> Tarjan<'a> {
    fn new(edges: &'a [Vec<usize>]) -> Self {
        Self {
            edges,
            next_index: 0,
            indices: vec![None; edges.len()],
            lowlinks: vec![0; edges.len()],
            stack: Vec::new(),
            on_stack: vec![false; edges.len()],
            components: Vec::new(),
        }
    }

    fn visit(&mut self, node: usize) {
        let index = self.next_index;
        self.next_index += 1;
        self.indices[node] = Some(index);
        self.lowlinks[node] = index;
        self.stack.push(node);
        self.on_stack[node] = true;

        for &target in &self.edges[node] {
            if self.indices[target].is_none() {
                self.visit(target);
                self.lowlinks[node] = self.lowlinks[node].min(self.lowlinks[target]);
            } else if self.on_stack[target] {
                self.lowlinks[node] = self.lowlinks[node]
                    .min(self.indices[target].expect("an indexed stack node has an index"));
            }
        }

        if self.lowlinks[node] == self.indices[node].expect("visited node has an index") {
            let mut component = Vec::new();
            loop {
                let member = self.stack.pop().expect("root node is on the stack");
                self.on_stack[member] = false;
                component.push(member);
                if member == node {
                    break;
                }
            }
            self.components.push(component);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::analyze_inheritance_graph;

    fn graph(edges: &[(char, &[char])]) -> BTreeMap<char, Vec<char>> {
        edges
            .iter()
            .map(|(node, targets)| (*node, targets.to_vec()))
            .collect()
    }

    fn cycles(edges: &[(char, &[char])]) -> Vec<Vec<char>> {
        analyze_inheritance_graph(&graph(edges))
            .cycles
            .into_iter()
            .map(|cycle| cycle.path)
            .collect()
    }

    #[test]
    fn analyzes_an_acyclic_chain_in_dependency_order() {
        let result = analyze_inheritance_graph(&graph(&[('a', &['b']), ('b', &['c']), ('c', &[])]));
        assert!(result.cycles.is_empty());
        assert_eq!(result.dependency_order, ['c', 'b', 'a']);
        assert!(result.cyclic_nodes.is_empty());
    }

    #[test]
    fn reports_a_self_cycle() {
        let result = analyze_inheritance_graph(&graph(&[('a', &['a'])]));
        assert_eq!(cycles(&[('a', &['a'])]), vec![vec!['a', 'a']]);
        assert!(result.cyclic_nodes.contains(&'a'));
    }

    #[test]
    fn reports_two_and_three_node_cycles_from_the_smallest_node() {
        assert_eq!(
            cycles(&[('b', &['a']), ('a', &['b'])]),
            vec![vec!['a', 'b', 'a']]
        );
        assert_eq!(
            cycles(&[('c', &['a']), ('a', &['b']), ('b', &['c'])]),
            vec![vec!['a', 'b', 'c', 'a']]
        );
    }

    #[test]
    fn reports_independent_sccs_and_keeps_acyclic_dependants() {
        let result = analyze_inheritance_graph(&graph(&[
            ('a', &['b']),
            ('b', &['a']),
            ('c', &['d']),
            ('d', &['c']),
            ('e', &['a']),
            ('f', &[]),
        ]));
        assert_eq!(
            result.cycles,
            vec![
                super::CycleReport {
                    path: vec!['a', 'b', 'a']
                },
                super::CycleReport {
                    path: vec!['c', 'd', 'c']
                },
            ]
        );
        assert_eq!(result.dependency_order, ['a', 'b', 'c', 'd', 'e', 'f']);
        assert!(!result.cyclic_nodes.contains(&'e'));
    }

    #[test]
    fn canonical_path_uses_request_order_for_the_first_returning_path() {
        let first = cycles(&[
            ('a', &['c', 'b']),
            ('b', &['a']),
            ('c', &['d']),
            ('d', &['a']),
        ]);
        let same_first_path = cycles(&[
            ('d', &['a']),
            ('c', &['d']),
            ('b', &['a']),
            ('a', &['c', 'b']),
        ]);
        assert_eq!(first, vec![vec!['a', 'c', 'd', 'a']]);
        assert_eq!(first, same_first_path);
    }

    #[test]
    fn dependency_order_places_a_node_after_a_cyclic_dependency() {
        let result = analyze_inheritance_graph(&graph(&[
            ('a', &['b']),
            ('b', &['c']),
            ('c', &['b']),
            ('d', &['a']),
        ]));
        assert_eq!(result.dependency_order, ['b', 'c', 'a', 'd']);
        assert_eq!(result.cycles[0].path, vec!['b', 'c', 'b']);
    }
}
