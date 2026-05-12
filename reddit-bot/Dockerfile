# ContractGateBot — single-process container
# Target: 1 vCPU / 512 MB

FROM python:3.11-slim

WORKDIR /app

# Install dependencies first (layer cache)
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy source
COPY . .

# Create DB directory
RUN mkdir -p /app/db

# Non-root user for security
RUN useradd -r -s /bin/false botuser && chown -R botuser /app
USER botuser

# Environment variables must be supplied at runtime (via --env-file or -e flags)
# Do NOT bake secrets into this image.

CMD ["python", "main.py"]
