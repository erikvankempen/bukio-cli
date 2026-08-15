/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// ECB reference-rate fetch (Phase 5 extension, user-requested).
// The ECB publishes daily euro reference rates: 1 EUR = N units of foreign
// currency — exactly bukio's convention. Endpoint: SDMX 2.1 data API
//   https://data-api.ecb.europa.eu/service/data/EXR/D.<CUR>.EUR.SP00.A?startPeriod=..&endPeriod=..
// No observations on weekends/holidays — we take the last observation on or
// before the requested date (same "latest on/before" semantics as getFxRate).
import { XMLParser } from 'fast-xml-parser';
import { fxError } from './index.js';

const BASE = 'https://data-api.ecb.europa.eu/service/data/EXR';
const WINDOW_DAYS = 10;

// test seam: override the network fetcher (default: global fetch)
let fetcherOverride = null;
export function setEcbFetcher(fn) { fetcherOverride = fn; }
export function clearEcbFetcher() { fetcherOverride = null; }

function addDays(isoDate, days) {
  const d = new Date(`${isoDate}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

/**
 * Fetch the ECB reference rate for `currency` on or before `date`.
 * Returns { date, rateX10000 } where date is the observation date (the
 * business day the rate was published) — or null when the currency is not in
 * the ECB reference set. Throws ECB_FETCH_FAILED on network/HTTP errors.
 * `fetcher` is injectable for tests (default: global fetch).
 */
export async function fetchEcbRate({ currency, date, fetcher = fetch }) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) {
    throw fxError('INVALID_DATE', `date '${date}' must be YYYY-MM-DD`);
  }
  const d = new Date(`${date}T00:00:00Z`);
  if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== date) {
    throw fxError('INVALID_DATE', `date '${date}' is not a valid calendar date`);
  }
  const from = addDays(date, -WINDOW_DAYS);
  const url = `${BASE}/D.${currency}.EUR.SP00.A?startPeriod=${from}&endPeriod=${date}`;
  let res;
  try {
    res = await (fetcherOverride ?? fetcher)(url);
  } catch (err) {
    throw fxError('ECB_FETCH_FAILED', `ECB unreachable for ${currency}: ${err.message}`);
  }
  if (!res.ok) {
    if (res.status === 404) return null; // currency not in the ECB reference set
    throw fxError('ECB_FETCH_FAILED', `ECB returned HTTP ${res.status} for ${currency}`);
  }
  const xml = await res.text();
  const observations = parseSdmxObservations(xml, currency);
  if (!observations.length) return null;
  const best = observations.filter((o) => o.date <= date).pop() ?? observations[observations.length - 1];
  return { date: best.date, rateX10000: Math.round(best.rate * 10000) };
}

/** Parse the SDMX-ML GenericData response into [{ date, rate }] (ascending). */
export function parseSdmxObservations(xml, currency) {
  const parser = new XMLParser({ removeNSPrefix: true, ignoreAttributes: false, attributeNamePrefix: '@_' });
  const doc = parser.parse(xml);
  const dataSet = doc?.GenericData?.DataSet ?? doc?.DataSet;
  if (!dataSet) return [];
  const series = Array.isArray(dataSet.Series) ? dataSet.Series : dataSet.Series ? [dataSet.Series] : [];
  const out = [];
  for (const s of series) {
    const obs = Array.isArray(s.Obs) ? s.Obs : s.Obs ? [s.Obs] : [];
    for (const o of obs) {
      const date = o?.ObsDimension?.['@_value'] ?? null;
      const value = o?.ObsValue?.['@_value'] ?? null;
      if (!date || value == null) continue;
      const rate = Number(value);
      if (!Number.isFinite(rate) || rate <= 0) continue;
      out.push({ date, rate });
    }
  }
  out.sort((a, b) => a.date.localeCompare(b.date));
  return out;
}
