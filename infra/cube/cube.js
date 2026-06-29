// Lumen — Cube.dev configuration.
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
