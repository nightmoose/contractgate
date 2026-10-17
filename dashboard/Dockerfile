FROM node:20-alpine AS deps
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm ci

FROM node:20-alpine AS builder
WORKDIR /app
COPY --from=deps /app/node_modules ./node_modules
COPY . .

# Build args are inlined by Next.js at build time (NEXT_PUBLIC_* prefix).
ARG NEXT_PUBLIC_DEMO_MODE=""
ARG NEXT_PUBLIC_API_URL="http://localhost:8080"
ARG NEXT_PUBLIC_API_KEY=""
ARG NEXT_PUBLIC_SUPABASE_URL=""
ARG NEXT_PUBLIC_SUPABASE_ANON_KEY=""
ARG NEXT_PUBLIC_APP_URL="http://localhost:3000"

# Write to .env.production so `next build` picks them up.
RUN echo "NEXT_PUBLIC_DEMO_MODE=${NEXT_PUBLIC_DEMO_MODE}" >> .env.production \
 && echo "NEXT_PUBLIC_API_URL=${NEXT_PUBLIC_API_URL}"     >> .env.production \
 && echo "NEXT_PUBLIC_API_KEY=${NEXT_PUBLIC_API_KEY}"     >> .env.production \
 && echo "NEXT_PUBLIC_SUPABASE_URL=${NEXT_PUBLIC_SUPABASE_URL}" >> .env.production \
 && echo "NEXT_PUBLIC_SUPABASE_ANON_KEY=${NEXT_PUBLIC_SUPABASE_ANON_KEY}" >> .env.production \
 && echo "NEXT_PUBLIC_APP_URL=${NEXT_PUBLIC_APP_URL}" >> .env.production

RUN npm run build

FROM node:20-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production
COPY --from=builder /app/public ./public
COPY --from=builder /app/.next/standalone ./
COPY --from=builder /app/.next/static ./.next/static

EXPOSE 3000
CMD ["node", "server.js"]
