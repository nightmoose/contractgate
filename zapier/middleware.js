const addApiKey = (request, z, bundle) => {
  request.headers = request.headers || {};
  if (bundle.authData && bundle.authData.api_key) {
    request.headers["X-Api-Key"] = bundle.authData.api_key;
  }
  request.headers.Accept = "application/json";
  request.headers["User-Agent"] = "contractgate-zapier/1.0.0";
  return request;
};

module.exports = { addApiKey };
