/// Detect the architectural layer of a file based on its path.
/// Returns a human-readable layer name.
pub fn detect_layer(file_path: &str) -> Option<String> {
    let path_lower = file_path.to_lowercase();
    let parts: Vec<&str> = path_lower.split('/').collect();

    // Check directory names and file patterns
    for part in &parts {
        match *part {
            // API / HTTP layer
            "api" | "routes" | "endpoints" | "handlers" | "controllers" | "views" | "rest"
            | "graphql" | "grpc" => {
                return Some("api".to_string());
            }
            // Service / Business logic
            "service" | "services" | "domain" | "business" | "logic" | "usecases"
            | "use_cases" | "interactors" => {
                return Some("service".to_string());
            }
            // Data / Persistence
            "data" | "database" | "db" | "models" | "entities" | "repository"
            | "repositories" | "persistence" | "migrations" | "schema" | "dao" => {
                return Some("data".to_string());
            }
            // UI / Frontend
            "ui" | "components" | "pages" | "layouts" | "widgets" | "templates" | "frontend"
            | "client" | "app" => {
                return Some("ui".to_string());
            }
            // Middleware / Infrastructure
            "middleware" | "interceptors" | "filters" | "pipes" | "guards" => {
                return Some("middleware".to_string());
            }
            // Utilities / Shared
            "util" | "utils" | "helpers" | "common" | "shared" | "lib" | "pkg" | "internal" => {
                return Some("util".to_string());
            }
            // Testing
            "test" | "tests" | "spec" | "specs" | "__tests__" | "testing" | "fixtures" => {
                return Some("test".to_string());
            }
            // Configuration
            "config" | "configuration" | "settings" | "env" => {
                return Some("config".to_string());
            }
            // Infrastructure / DevOps
            "infra" | "infrastructure" | "deploy" | "ci" | "docker" | "k8s" | "terraform" => {
                return Some("infra".to_string());
            }
            _ => {}
        }
    }

    // Check file-level patterns (last segment, strip extension first)
    let filename = parts.last().unwrap_or(&"");
    let stem = if let Some(dot_pos) = filename.rfind('.') {
        &filename[..dot_pos]
    } else {
        filename
    };

    if stem.contains("test") || stem.contains("spec") {
        return Some("test".to_string());
    }
    if stem.contains("config") || stem.contains("settings") {
        return Some("config".to_string());
    }
    if stem.ends_with("controller") || stem.ends_with("handler") {
        return Some("api".to_string());
    }
    if stem.ends_with("service") {
        return Some("service".to_string());
    }
    if stem.ends_with("model") || stem.ends_with("entity") || stem.ends_with("repository") {
        return Some("data".to_string());
    }

    None // Unknown layer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_layer() {
        assert_eq!(detect_layer("/project/src/routes/users.rs"), Some("api".to_string()));
        assert_eq!(detect_layer("/project/src/handlers/auth.py"), Some("api".to_string()));
        assert_eq!(detect_layer("/project/src/controllers/payment.ts"), Some("api".to_string()));
    }

    #[test]
    fn test_service_layer() {
        assert_eq!(detect_layer("/project/src/services/user_service.py"), Some("service".to_string()));
        assert_eq!(detect_layer("/project/src/domain/billing.rs"), Some("service".to_string()));
        assert_eq!(detect_layer("/project/src/payment.service.ts"), Some("service".to_string()));
    }

    #[test]
    fn test_data_layer() {
        assert_eq!(detect_layer("/project/src/repository/user_repo.py"), Some("data".to_string()));
        assert_eq!(detect_layer("/project/src/models/user.rb"), Some("data".to_string()));
        assert_eq!(detect_layer("/project/src/migrations/001_init.sql"), Some("data".to_string()));
    }

    #[test]
    fn test_test_layer() {
        assert_eq!(detect_layer("/project/tests/test_auth.py"), Some("test".to_string()));
        assert_eq!(detect_layer("/project/src/__tests__/utils.ts"), Some("test".to_string()));
        assert_eq!(detect_layer("/project/src/auth_test.go"), Some("test".to_string()));
    }

    #[test]
    fn test_config_layer() {
        assert_eq!(detect_layer("/project/src/config/database.rs"), Some("config".to_string()));
        assert_eq!(detect_layer("/project/src/app_settings.py"), Some("config".to_string()));
    }

    #[test]
    fn test_util_layer() {
        assert_eq!(detect_layer("/project/src/utils/string_helpers.py"), Some("util".to_string()));
        assert_eq!(detect_layer("/project/src/helpers/date.ts"), Some("util".to_string()));
    }

    #[test]
    fn test_unknown_layer() {
        assert_eq!(detect_layer("/project/src/main.rs"), None);
        assert_eq!(detect_layer("/project/src/lib.rs"), None);
    }
}
