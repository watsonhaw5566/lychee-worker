# -----------------------------------------------------------------------------
# Stage 1 — Rust builder: compiles the lychee_worker PHP extension
# -----------------------------------------------------------------------------
FROM rust:1.75-bookworm AS builder

WORKDIR /app

# Pre-fetch cargo index to cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p rust/src && echo "fn main() {}" > rust/src/lib.rs
RUN cargo build --release 2>/dev/null || true

# Now copy the actual source and rebuild
COPY . .
RUN cargo build --release

# -----------------------------------------------------------------------------
# Stage 2 — Runtime: PHP 8.3 + the prebuilt extension
# -----------------------------------------------------------------------------
FROM php:8.3-cli-bookworm

# System dependencies
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        curl \
        git \
        unzip \
        libpng-dev \
        libonig-dev \
        libxml2-dev \
        zip \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Install Composer
COPY --from=composer:2.7 /usr/bin/composer /usr/bin/composer

# Determine the PHP extension directory dynamically and install the extension
RUN EXT_DIR=$(php-config --extension-dir) \
    && echo "[build] PHP extension directory: ${EXT_DIR}"

# Copy the prebuilt extension from the builder stage. The cargo build
# produces liblychee_worker.so but PHP expects it to be named lychee_worker.so
COPY --from=builder /app/target/release/liblychee_worker.so /tmp/liblychee_worker.so
RUN EXT_DIR=$(php-config --extension-dir) \
    && cp /tmp/liblychee_worker.so "${EXT_DIR}/lychee_worker.so" \
    && rm /tmp/liblychee_worker.so \
    && echo "extension=lychee_worker.so" > /usr/local/etc/php/conf.d/99-lychee_worker.ini

# Verify the extension loads — fail the build if it does not
RUN php -m 2>&1 | grep -q "^lychee_worker$" \
    && echo "[build] ✅ lychee_worker extension loaded successfully" \
    || (echo "[build] ❌ lychee_worker extension failed to load"; php -m; exit 1)

# Copy project files
WORKDIR /var/www/html
COPY . /var/www/html

# Install PHP dependencies
RUN composer install --no-dev --optimize-autoloader --no-interaction

# Final verification
RUN php -r "
    if (!extension_loaded('lychee_worker')) {
        echo \"❌ extension not loaded\n\";
        exit(1);
    }
    echo \"✅ lychee_worker extension: loaded\n\";
    echo \"✅ PHP version: \" . phpversion() . \"\n\";
    echo \"✅ Ready to use\n\";
"

EXPOSE 8000

# Default to a simple verify command; override this in docker-compose or your
# orchestration layer to run `php think worker` in your actual ThinkPHP project.
CMD ["php", "-r", "echo \"lychee-worker ready!\\n\"; print_r(lychee_worker_stats());"]