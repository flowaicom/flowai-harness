async function getJson(path) {
  const response = await fetch(path);
  if (!response.ok) {
    throw new Error(`${path} failed with ${response.status}`);
  }
  return response.json();
}

function renderSummary(summary) {
  document.querySelector("#summary").innerHTML = `
    <div><span>Products</span><strong>${summary.products}</strong></div>
    <div><span>Actions</span><strong>${summary.actions}</strong></div>
    <div><span>Low Stock</span><strong>${summary.low_stock_products}</strong></div>
  `;
}

function renderProducts(payload) {
  document.querySelector("#products").innerHTML = payload.products
    .map(
      (product) => `
        <div class="row">
          <div>
            <strong>${product.product_id}</strong>
            <span>${product.product_name}</span>
          </div>
          <div class="numbers">
            <span>On hand ${product.on_hand}</span>
            <span>Safety ${product.safety_stock}</span>
            <span>Holdback ${product.holdback_units}</span>
          </div>
        </div>
      `,
    )
    .join("");
}

function renderActions(payload) {
  document.querySelector("#actions").innerHTML =
    payload.actions.length === 0
      ? '<p class="empty">No actions recorded.</p>'
      : payload.actions
          .map(
            (action) => `
              <div class="row">
                <div>
                  <strong>${action.action_type}</strong>
                  <span>${action.reason}</span>
                </div>
                <div class="numbers">
                  <span>${action.product_count} products</span>
                  <span>${action.priority}</span>
                </div>
              </div>
            `,
          )
          .join("");
}

async function refresh() {
  const [summary, products, actions] = await Promise.all([
    getJson("/state/summary"),
    getJson("/products?limit=20"),
    getJson("/actions"),
  ]);
  renderSummary(summary);
  renderProducts(products);
  renderActions(actions);
}

document.querySelector("#refresh").addEventListener("click", refresh);
refresh().catch((error) => {
  document.querySelector("#summary").innerHTML = `<p class="error">${error.message}</p>`;
});
