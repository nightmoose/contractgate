import assert from "node:assert/strict";
import test from "node:test";
import {
  checkoutPrice,
  freePlanFor,
  isPaidStoredPlan,
  paidPlanForSubscription,
  planForCheckout,
} from "./billing-plan";

const all = { pro: ["price_pro_m", "price_pro_y"], impact: ["price_imp_m", "price_imp_y"] };
const noImpact = { pro: ["price_pro_m", "price_pro_y"], impact: [undefined, undefined] };
const noPro = { pro: [undefined, undefined], impact: ["price_imp_m", "price_imp_y"] };

test("a Pro + Impact price grants impact on checkout", () => {
  assert.equal(planForCheckout("price_imp_m", all, { aisg: false }), "impact");
  assert.equal(planForCheckout("price_imp_y", all, { aisg: false }), "impact");
  // The price decides, even when Pro prices are unset (lenient Pro rule).
  assert.equal(planForCheckout("price_imp_y", noPro, { aisg: false }), "impact");
});

test("Pro checkout mapping is unchanged", () => {
  assert.equal(planForCheckout("price_pro_m", all, { aisg: false }), "growth");
  assert.equal(planForCheckout("price_pro_y", all, { aisg: true }), "aisg_pro");
  assert.equal(planForCheckout(undefined, all, { aisg: false }), "growth");
  assert.equal(planForCheckout("price_other", all, { aisg: false }), null);
  // Pro env unset: any non-impact price is accepted as Pro (existing behaviour).
  assert.equal(planForCheckout("price_other", noPro, { aisg: false }), "growth");
});

test("impact price ids that are not configured never grant impact", () => {
  assert.equal(planForCheckout("price_imp_m", noImpact, { aisg: false }), null);
});

test("subscription updates keep impact orgs on impact", () => {
  assert.equal(paidPlanForSubscription("impact", "price_imp_m", all), "impact");
  assert.equal(paidPlanForSubscription("impact", undefined, all), "impact");
  assert.equal(paidPlanForSubscription("impact", "price_unknown", noImpact), "impact");
});

test("subscription updates follow a plan switch made in Stripe", () => {
  assert.equal(paidPlanForSubscription("growth", "price_imp_y", all), "impact");
  assert.equal(paidPlanForSubscription("impact", "price_pro_m", all), "growth");
  assert.equal(paidPlanForSubscription("aisg_pro", "price_pro_m", all), "aisg_pro");
});

test("subscription updates without a known price keep the product line", () => {
  assert.equal(paidPlanForSubscription("growth", undefined, all), "growth");
  assert.equal(paidPlanForSubscription("free", undefined, all), "growth");
  assert.equal(paidPlanForSubscription("aisg_free", undefined, all), "aisg_pro");
});

test("cancelled impact goes to free", () => {
  assert.equal(freePlanFor("impact"), "free");
  assert.equal(freePlanFor("aisg_pro"), "aisg_free");
  assert.equal(freePlanFor("growth"), "free");
});

test("paid stored plans include impact", () => {
  for (const p of ["growth", "aisg_pro", "impact", "enterprise"]) assert.ok(isPaidStoredPlan(p));
  for (const p of ["free", "aisg_free", null, undefined]) assert.ok(!isPaidStoredPlan(p));
});

test("checkout picks the price for the plan and period", () => {
  const prices = { proMonthly: "pm", proAnnual: "py", impactMonthly: "im", impactAnnual: "iy" };
  assert.equal(checkoutPrice("impact", false, prices), "im");
  assert.equal(checkoutPrice("impact", true, prices), "iy");
  assert.equal(checkoutPrice("pro", false, prices), "pm");
  assert.equal(checkoutPrice("pro", true, prices), "py");
  assert.equal(checkoutPrice("impact", true, { proAnnual: "py" }), undefined);
});
