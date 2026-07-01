-- Lumen — TPC-H sample data for ClickHouse, self-contained (no dbgen required).
-- Same shape/column names/row counts as ../postgres/seed.sql so the two
-- warehouses are interchangeable behind Cube — but the generation SQL is
-- ClickHouse-native (numbers() instead of generate_series, ARRAY JOIN instead
-- of CROSS JOIN LATERAL, rand()/4294967296 instead of random(), no FK
-- constraints — ClickHouse doesn't have them).
-- Idempotent: drops and recreates. Yields ~5k orders / ~20k lineitems.

DROP TABLE IF EXISTS lineitem;
DROP TABLE IF EXISTS orders;
DROP TABLE IF EXISTS customer;
DROP TABLE IF EXISTS nation;
DROP TABLE IF EXISTS region;

CREATE TABLE region (
    r_regionkey Int32,
    r_name      String,
    r_comment   String
) ENGINE = MergeTree ORDER BY r_regionkey;

CREATE TABLE nation (
    n_nationkey Int32,
    n_name      String,
    n_regionkey Int32,
    n_comment   String
) ENGINE = MergeTree ORDER BY n_nationkey;

CREATE TABLE customer (
    c_custkey    Int32,
    c_name       String,
    c_address    String,
    c_nationkey  Int32,
    c_phone      String,
    c_acctbal    Decimal(15,2),
    c_mktsegment String,
    c_comment    String
) ENGINE = MergeTree ORDER BY c_custkey;

CREATE TABLE orders (
    o_orderkey      Int32,
    o_custkey       Int32,
    o_orderstatus   String,
    o_totalprice    Decimal(15,2),
    o_orderdate     Date,
    o_orderpriority String,
    o_clerk         String,
    o_shippriority  Int32,
    o_comment       String
) ENGINE = MergeTree ORDER BY o_orderkey;

CREATE TABLE lineitem (
    l_orderkey      Int32,
    l_partkey       Int32,
    l_suppkey       Int32,
    l_linenumber    Int32,
    l_quantity      Decimal(15,2),
    l_extendedprice Decimal(15,2),
    l_discount      Decimal(15,2),
    l_tax           Decimal(15,2),
    l_returnflag    String,
    l_linestatus    String,
    l_shipdate      Date,
    l_commitdate    Date,
    l_receiptdate   Date,
    l_shipinstruct  String,
    l_shipmode      String,
    l_comment       String
) ENGINE = MergeTree ORDER BY (l_orderkey, l_linenumber);

-- 5 real regions
INSERT INTO region VALUES
 (0,'AFRICA',''),(1,'AMERICA',''),(2,'ASIA',''),(3,'EUROPE',''),(4,'MIDDLE EAST','');

-- 25 real nations
INSERT INTO nation VALUES
 (0,'ALGERIA',0,''),(1,'ARGENTINA',1,''),(2,'BRAZIL',1,''),(3,'CANADA',1,''),(4,'EGYPT',4,''),
 (5,'ETHIOPIA',0,''),(6,'FRANCE',3,''),(7,'GERMANY',3,''),(8,'INDIA',2,''),(9,'INDONESIA',2,''),
 (10,'IRAN',4,''),(11,'IRAQ',4,''),(12,'JAPAN',2,''),(13,'JORDAN',4,''),(14,'KENYA',0,''),
 (15,'MOROCCO',0,''),(16,'MOZAMBIQUE',0,''),(17,'PERU',1,''),(18,'CHINA',2,''),(19,'ROMANIA',3,''),
 (20,'SAUDI ARABIA',4,''),(21,'VIETNAM',2,''),(22,'RUSSIA',3,''),(23,'UNITED KINGDOM',3,''),(24,'UNITED STATES',1,'');

-- 1,500 customers
INSERT INTO customer
SELECT
    g,
    concat('Customer#', leftPad(toString(g), 9, '0')),
    lower(hex(MD5(toString(g)))),
    toInt32(rand() % 25),
    concat(leftPad(toString(10 + rand() % 25), 2, '0'), '-',
           leftPad(toString(100 + rand() % 900), 3, '0'), '-',
           leftPad(toString(100 + rand() % 900), 3, '0')),
    round(rand() / 4294967296 * 11000 - 1000, 2),
    ['BUILDING','AUTOMOBILE','MACHINERY','HOUSEHOLD','FURNITURE'][toUInt32(rand() % 5) + 1],
    'seed'
FROM (SELECT number + 1 AS g FROM numbers(1500));

-- 5,000 orders across 1992-01-01 .. 1998-12-31
INSERT INTO orders
SELECT
    g,
    toInt32(1 + rand() % 1500),
    ['O','F','P'][toUInt32(rand() % 3) + 1],
    round(rand() / 4294967296 * 200000 + 1000, 2),
    toDate('1992-01-01') + toUInt32(rand() % 2557),
    ['1-URGENT','2-HIGH','3-MEDIUM','4-NOT SPECIFIED','5-LOW'][toUInt32(rand() % 5) + 1],
    concat('Clerk#', leftPad(toString(1 + rand() % 1000), 9, '0')),
    0,
    'seed'
FROM (SELECT number + 1 AS g FROM numbers(5000));

-- 1-7 lineitems per order (~20k rows), fanned out via ARRAY JOIN (ClickHouse's
-- equivalent of Postgres' CROSS JOIN LATERAL generate_series).
INSERT INTO lineitem
SELECT
    o.o_orderkey,
    toInt32(1 + rand() % 2000),
    toInt32(1 + rand() % 100),
    ln + 1,
    round(rand() / 4294967296 * 49 + 1, 2),
    round(rand() / 4294967296 * 40000 + 900, 2),
    round(toUInt32(rand() % 11) / 100.0, 2),
    round(toUInt32(rand() % 9) / 100.0, 2),
    ['A','N','R'][toUInt32(rand() % 3) + 1],
    ['O','F'][toUInt32(rand() % 2) + 1],
    o.o_orderdate + toUInt32(rand() % 120),
    o.o_orderdate + toUInt32(rand() % 90),
    o.o_orderdate + toUInt32(rand() % 120),
    'DELIVER IN PERSON',
    ['TRUCK','MAIL','SHIP','RAIL','AIR','REG AIR','FOB'][toUInt32(rand() % 7) + 1],
    'seed'
FROM orders AS o
ARRAY JOIN range(1 + (o.o_orderkey % 7)) AS ln;
