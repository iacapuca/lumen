// Lumen — Cube.dev configuration (ClickHouse warehouse instance).
// Mirrors ../cube/cube.js. Kept as a separate file (not a symlink) because the
// two Cube instances are independent deployables that may diverge.
//
// The JWT Lumen mints carries the security context as its payload; Cube exposes
// it as `securityContext`. Row-level security belongs HERE (the semantic layer),
// not in Lumen. The runtime always forwards the security context; this is where
// you turn it into mandatory filters.
//
// Example (uncomment + adapt to your tenant column):
//
// module.exports = {
//   queryRewrite: (query, { securityContext }) => {
//     if (securityContext && securityContext.tenant_id) {
//       query.filters.push({
//         member: 'orders.tenant_id',
//         operator: 'equals',
//         values: [securityContext.tenant_id],
//       });
//     }
//     return query;
//   },
// };

module.exports = {};
