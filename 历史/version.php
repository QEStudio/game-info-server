<?php

define('DATA_DIR', __DIR__ . '/../data');
define('IMAGES_DIR', DATA_DIR . '/images');
define('USERS_DIR', DATA_DIR . '/users');
define('CACHE_DIR', DATA_DIR . '/cache');
define('STATS_FILE', CACHE_DIR . '/stats.json');
define('PENDING_FILE', CACHE_DIR . '/pending.json');
define('VERSION_CACHE_DIR', CACHE_DIR . '/versions');
define('GEOIP_CACHE_DIR', CACHE_DIR . '/geoip');
define('GEOIP_QUEUE_FILE', CACHE_DIR . '/geoip_queue.json');
define('STATS_AGGREGATE_FILE', CACHE_DIR . '/stats_aggregate.json');
define('STATS_AGGREGATE_TTL', 300);
define('FLUSH_INTERVAL', 10);

header('Content-Type: application/json; charset=utf-8');

function sendResponse($code, $data = null, $message = '') {
    http_response_code($code);
    $response = array('code' => $code);
    if ($data !== null) {
        $response['data'] = $data;
    }
    if ($message !== '') {
        $response['message'] = $message;
    }
    echo json_encode($response, JSON_UNESCAPED_UNICODE);
    exit;
}

function sendErrorResponse($code, $message) {
    sendResponse($code, null, $message);
}

function getIpAddress() {
    $ip = '';
    if (!empty($_SERVER['HTTP_CLIENT_IP'])) {
        $ip = $_SERVER['HTTP_CLIENT_IP'];
    } elseif (!empty($_SERVER['HTTP_X_FORWARDED_FOR'])) {
        $ips = explode(',', $_SERVER['HTTP_X_FORWARDED_FOR']);
        $ip = trim($ips[0]);
    } elseif (!empty($_SERVER['REMOTE_ADDR'])) {
        $ip = $_SERVER['REMOTE_ADDR'];
    }
    return filter_var($ip, FILTER_VALIDATE_IP) ? $ip : 'unknown';
}

function isValidUUID($uuid) {
    if (empty($uuid)) {
        return false;
    }
    $pattern = '/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i';
    return preg_match($pattern, $uuid) === 1;
}

function versionCompare($v1, $v2) {
    $parts1 = array_map('intval', explode('.', $v1));
    $parts2 = array_map('intval', explode('.', $v2));
    
    $maxLen = max(count($parts1), count($parts2));
    $parts1 = array_pad($parts1, $maxLen, 0);
    $parts2 = array_pad($parts2, $maxLen, 0);
    
    for ($i = 0; $i < $maxLen; $i++) {
        if ($parts1[$i] > $parts2[$i]) return 1;
        if ($parts1[$i] < $parts2[$i]) return -1;
    }
    return 0;
}

function getLatestGameData($customId) {
    $customId = (int)$customId;
    if ($customId <= 0) {
        return null;
    }
    
    $customDir = IMAGES_DIR . '/' . $customId;
    if (!is_dir($customDir)) {
        return null;
    }
    
    if (!is_dir(VERSION_CACHE_DIR)) {
        mkdir(VERSION_CACHE_DIR, 0755, true);
    }
    
    $cacheFile = VERSION_CACHE_DIR . '/' . $customId . '.json';
    $customDirMtime = filemtime($customDir);
    
    if (file_exists($cacheFile)) {
        $cacheMtime = filemtime($cacheFile);
        if ($cacheMtime >= $customDirMtime) {
            $cached = json_decode(file_get_contents($cacheFile), true);
            if ($cached && isset($cached['version_name']) && isset($cached['data'])) {
                return $cached;
            }
        }
    }
    
    $versions = array();
    $dirs = scandir($customDir);
    foreach ($dirs as $dir) {
        if ($dir === '.' || $dir === '..') continue;
        
        $dataFile = $customDir . '/' . $dir . '/data.json';
        if (file_exists($dataFile)) {
            $content = file_get_contents($dataFile);
            $data = json_decode($content, true);
            
            if (json_last_error() === JSON_ERROR_NONE) {
                $versions[] = array(
                    'version_name' => $dir,
                    'data' => $data
                );
            }
        }
    }
    
    if (empty($versions)) {
        return null;
    }
    
    usort($versions, function($a, $b) {
        return versionCompare($b['version_name'], $a['version_name']);
    });
    
    $result = $versions[0];
    file_put_contents($cacheFile, json_encode($result), LOCK_EX);
    
    return $result;
}

function getIpSegment($ip) {
    $parts = explode('.', $ip);
    if (count($parts) >= 2) {
        return $parts[0] . '.' . $parts[1] . '.*.*';
    }
    return 'unknown';
}

function getGeoLocation($ip) {
    if ($ip === 'unknown' || $ip === '127.0.0.1' || strpos($ip, '192.168.') === 0 || strpos($ip, '10.') === 0 || strpos($ip, '172.') === 0) {
        return array(
            'country' => 'Local',
            'countryCode' => 'LOCAL',
            'city' => 'Local Network',
            'lat' => 0,
            'lon' => 0,
            'timezone' => 'UTC'
        );
    }
    
    if (!is_dir(GEOIP_CACHE_DIR)) {
        mkdir(GEOIP_CACHE_DIR, 0755, true);
    }
    
    $cacheFile = GEOIP_CACHE_DIR . '/' . md5($ip) . '.json';
    
    if (file_exists($cacheFile)) {
        $cached = json_decode(file_get_contents($cacheFile), true);
        if ($cached && isset($cached['country'])) {
            return $cached;
        }
    }
    
    queueGeoIpLookup($ip);
    
    return array(
        'country' => 'Unknown',
        'countryCode' => 'XX',
        'city' => 'Unknown',
        'lat' => 0,
        'lon' => 0,
        'timezone' => 'UTC'
    );
}

function queueGeoIpLookup($ip) {
    if (!is_dir(CACHE_DIR)) {
        mkdir(CACHE_DIR, 0755, true);
    }
    
    $queue = array();
    if (file_exists(GEOIP_QUEUE_FILE)) {
        $queue = json_decode(file_get_contents(GEOIP_QUEUE_FILE), true);
        if (!is_array($queue)) {
            $queue = array();
        }
    }
    
    if (!in_array($ip, $queue)) {
        $queue[] = $ip;
        file_put_contents(GEOIP_QUEUE_FILE, json_encode($queue), LOCK_EX);
    }
}

function processGeoIpQueue($maxItems = 10) {
    if (!file_exists(GEOIP_QUEUE_FILE)) {
        return;
    }
    
    $queue = json_decode(file_get_contents(GEOIP_QUEUE_FILE), true);
    if (!is_array($queue) || empty($queue)) {
        return;
    }
    
    $processed = 0;
    $remaining = array();
    
    foreach ($queue as $ip) {
        if ($processed >= $maxItems) {
            $remaining[] = $ip;
            continue;
        }
        
        $cacheFile = GEOIP_CACHE_DIR . '/' . md5($ip) . '.json';
        if (file_exists($cacheFile)) {
            continue;
        }
        
        $url = 'http://ip-api.com/json/' . $ip . '?fields=status,country,countryCode,city,lat,lon,timezone';
        $response = @file_get_contents($url, false, stream_context_create(array(
            'http' => array(
                'timeout' => 2,
                'method' => 'GET'
            )
        )));
        
        if ($response) {
            $data = json_decode($response, true);
            if ($data && isset($data['status']) && $data['status'] === 'success') {
                $geoData = array(
                    'country' => isset($data['country']) ? $data['country'] : 'Unknown',
                    'countryCode' => isset($data['countryCode']) ? $data['countryCode'] : 'XX',
                    'city' => isset($data['city']) ? $data['city'] : 'Unknown',
                    'lat' => isset($data['lat']) ? (float)$data['lat'] : 0,
                    'lon' => isset($data['lon']) ? (float)$data['lon'] : 0,
                    'timezone' => isset($data['timezone']) ? $data['timezone'] : 'UTC'
                );
                file_put_contents($cacheFile, json_encode($geoData), LOCK_EX);
            } else {
                $defaultGeo = array(
                    'country' => 'Unknown',
                    'countryCode' => 'XX',
                    'city' => 'Unknown',
                    'lat' => 0,
                    'lon' => 0,
                    'timezone' => 'UTC'
                );
                file_put_contents($cacheFile, json_encode($defaultGeo), LOCK_EX);
            }
        }
        
        $processed++;
        usleep(200000);
    }
    
    if (!empty($remaining)) {
        file_put_contents(GEOIP_QUEUE_FILE, json_encode($remaining), LOCK_EX);
    } else {
        @unlink(GEOIP_QUEUE_FILE);
    }
}

function initStatsFile() {
    if (!is_dir(CACHE_DIR)) {
        mkdir(CACHE_DIR, 0755, true);
    }
    
    if (!file_exists(STATS_FILE)) {
        $default = array(
            'total_users' => 0,
            'total_access' => 0,
            'with_uuid' => 0,
            'without_uuid' => 0,
            'regions' => array(),
            'last_updated' => date('Y-m-d H:i:s')
        );
        file_put_contents(STATS_FILE, json_encode($default, JSON_PRETTY_PRINT), LOCK_EX);
    }
}

function getPendingCount() {
    if (!file_exists(PENDING_FILE)) {
        return 0;
    }
    
    $fp = fopen(PENDING_FILE, 'r');
    if (!$fp) return 0;
    
    flock($fp, LOCK_SH);
    $count = (int)fgets($fp);
    flock($fp, LOCK_UN);
    fclose($fp);
    
    return $count;
}

function incrementPendingCount() {
    if (!is_dir(CACHE_DIR)) {
        mkdir(CACHE_DIR, 0755, true);
    }
    
    $fp = fopen(PENDING_FILE, 'c+');
    if (!$fp) return;
    
    flock($fp, LOCK_EX);
    
    $count = (int)fgets($fp);
    $count++;
    
    ftruncate($fp, 0);
    rewind($fp);
    fwrite($fp, (string)$count);
    fflush($fp);
    
    flock($fp, LOCK_UN);
    fclose($fp);
    
    return $count;
}

function flushPendingStats() {
    $pendingCount = getPendingCount();
    if ($pendingCount <= 0) {
        return;
    }
    
    $stats = array();
    if (file_exists(STATS_FILE)) {
        $content = file_get_contents(STATS_FILE);
        $stats = json_decode($content, true);
    }
    
    if (!isset($stats['total_access'])) {
        $stats = array(
            'total_users' => 0,
            'total_access' => 0,
            'with_uuid' => 0,
            'without_uuid' => 0,
            'regions' => array(),
            'last_updated' => date('Y-m-d H:i:s')
        );
    }
    
    $stats['total_access'] += $pendingCount;
    $stats['last_updated'] = date('Y-m-d H:i:s');
    
    file_put_contents(STATS_FILE, json_encode($stats, JSON_PRETTY_PRINT), LOCK_EX);
    
    $fp = fopen(PENDING_FILE, 'c+');
    if ($fp) {
        flock($fp, LOCK_EX);
        ftruncate($fp, 0);
        rewind($fp);
        fwrite($fp, '0');
        flock($fp, LOCK_UN);
        fclose($fp);
    }
}

function recordAccess($ip, $uuid) {
    $hasUuid = !empty($uuid) && isValidUUID($uuid);
    $ipSegment = getIpSegment($ip);
    
    if ($hasUuid) {
        $targetDir = USERS_DIR . '/' . $ipSegment . '/' . $uuid;
    } else {
        $targetDir = USERS_DIR . '/public/' . $ipSegment;
    }
    
    $isNewUser = !is_dir($targetDir);
    
    if ($isNewUser) {
        mkdir($targetDir, 0755, true);
        $geo = getGeoLocation($ip);
        $geo['original_ip'] = $ip;
        $geoFile = $targetDir . '/geo.json';
        file_put_contents($geoFile, json_encode($geo), LOCK_EX);
    }
    
    $timesFile = $targetDir . '/times.txt';
    $now = date('Y-m-d H:i:s');
    
    if (file_exists($timesFile)) {
        $content = file_get_contents($timesFile);
        $parts = explode("\n", trim($content));
        
        $count = isset($parts[0]) ? (int)$parts[0] : 0;
        $firstTime = isset($parts[1]) ? $parts[1] : $now;
        
        $count++;
    } else {
        $count = 1;
        $firstTime = $now;
    }
    
    $newContent = sprintf("%d\n%s\n%s\n", $count, $firstTime, $now);
    file_put_contents($timesFile, $newContent, LOCK_EX);
    
    initStatsFile();
    
    $pendingCount = incrementPendingCount();
    
    if ($isNewUser) {
        $geo = getGeoLocation($ip);
        $stats = json_decode(file_get_contents(STATS_FILE), true);
        if ($stats) {
            $stats['total_users']++;
            if ($hasUuid) {
                $stats['with_uuid']++;
            } else {
                $stats['without_uuid']++;
            }
            $countryCode = $geo['countryCode'];
            if ($countryCode !== 'XX') {
                if (!isset($stats['regions'][$countryCode])) {
                    $stats['regions'][$countryCode] = array(
                        'country' => $geo['country'],
                        'count' => 0
                    );
                }
                $stats['regions'][$countryCode]['count']++;
            }
            $stats['last_updated'] = date('Y-m-d H:i:s');
            file_put_contents(STATS_FILE, json_encode($stats, JSON_PRETTY_PRINT), LOCK_EX);
        }
    }
    
    if ($pendingCount >= FLUSH_INTERVAL) {
        flushPendingStats();
    }
    
    return $count;
}

$requestUri = $_SERVER['REQUEST_URI'];
$path = parse_url($requestUri, PHP_URL_PATH);
$path = rtrim($path, '/');

if (preg_match('#/images/game_info/custom/new/(\d+)/data\.json$#', $path, $matches)) {
    $customId = $matches[1];
    
    $input = json_decode(file_get_contents('php://input'), true);
    $uuid = isset($input['uuid']) ? trim($input['uuid']) : '';
    
    $ip = getIpAddress();
    recordAccess($ip, $uuid);
    
    $result = getLatestGameData($customId);
    if ($result === null) {
        sendErrorResponse(404, 'Game data not found for custom_id: ' . $customId);
    }
    
    $gameData = $result['data'];
    $versionName = $result['version_name'];
    
    $gameInfo = array(
        'game_name' => isset($gameData['game_name']) ? $gameData['game_name'] : '',
        'kind_name' => isset($gameData['kind_name']) ? $gameData['kind_name'] : '',
        'version_name' => $versionName,
        'game_id' => isset($gameData['game_id']) ? (int)$gameData['game_id'] : 0,
        'version_id' => isset($gameData['version_id']) ? (int)$gameData['version_id'] : 0,
        'custom_id' => (int)$customId,
        'custom_version_id' => isset($gameData['custom_version_id']) ? (int)$gameData['custom_version_id'] : 0,
        'custom' => isset($gameData['custom']) ? (bool)$gameData['custom'] : true,
        'description' => isset($gameData['description']) ? $gameData['description'] : '',
        'link' => isset($gameData['link']) ? $gameData['link'] : ''
    );
    
    sendResponse(200, array('game_info' => $gameInfo));
}

sendErrorResponse(404, 'Not Found');
