#![feature(alloc_system)]
extern crate alloc_system;

extern crate env_logger;
extern crate futures;
extern crate rand;
extern crate rdkafka;

mod test_utils;

use futures::*;

use rdkafka::consumer::{Consumer, CommitMode};
use rdkafka::message::Timestamp;
use rdkafka::topic_partition_list::{Offset, TopicPartitionList};
use rdkafka::error::KafkaError;

use test_utils::*;

use std::time::{Duration, Instant};
use std::collections::HashMap;

// All messages should go to the same partition.
#[test]
fn test_produce_partition() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    let message_map = produce_messages(&topic_name, 100, &value_fn, &key_fn, Some(0), None);

    let res = message_map.iter()
        .filter(|&(&(partition, _), _)| partition == 0)
        .count();

    assert_eq!(res, 100);
}

// All produced messages should be consumed.
#[test]
fn test_produce_consume_base() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    let message_map = produce_messages(&topic_name, 100, &value_fn, &key_fn, None, None);
    let mut consumer = create_stream_consumer(&rand_test_group(), None);
    consumer.subscribe(&vec![topic_name.as_str()]).unwrap();

    let _consumer_future = consumer.start()
        .take(100)
        .for_each(|message| {
            match message {
                Ok(m) => {
                    let id = message_map.get(&(m.partition(), m.offset())).unwrap();
                    match m.timestamp() {
                        Timestamp::CreateTime(timestamp) => assert!(timestamp > 1489495183000),
                        _ => panic!("Expected createtime for message timestamp")
                    };
                    assert_eq!(m.payload_view::<str>().unwrap().unwrap(), value_fn(*id));
                    assert_eq!(m.key_view::<str>().unwrap().unwrap(), key_fn(*id));
                    assert_eq!(m.topic_name(), topic_name.as_str());
                },
                Err(e) => panic!("Error receiving message: {:?}", e)
            };
            Ok(())
        })
        .wait();
}

// All produced messages should be consumed.
#[test]
fn test_produce_consume_base_assign() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    produce_messages(&topic_name, 10, &value_fn, &key_fn, Some(0), None);
    produce_messages(&topic_name, 10, &value_fn, &key_fn, Some(1), None);
    produce_messages(&topic_name, 10, &value_fn, &key_fn, Some(2), None);
    let mut consumer = create_stream_consumer(&rand_test_group(), None);
    let mut tpl = TopicPartitionList::new();
    tpl.add_partition_offset(&topic_name, 0, Offset::Beginning);
    tpl.add_partition_offset(&topic_name, 1, Offset::Offset(2));
    tpl.add_partition_offset(&topic_name, 2, Offset::Offset(9));
    consumer.assign(&tpl).unwrap();

    let mut partition_count = vec![0, 0, 0];

    let _consumer_future = consumer.start()
        .take(19)
        .for_each(|message| {
            match message {
                Ok(m) => partition_count[m.partition() as usize] += 1,
                Err(e) => panic!("Error receiving message: {:?}", e)
            };
            Ok(())
        })
        .wait();

    assert_eq!(partition_count, vec![10, 8, 1]);
}

// All produced messages should be consumed.
#[test]
fn test_produce_consume_with_timestamp() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    let message_map = produce_messages(&topic_name, 100, &value_fn, &key_fn, Some(0), Some(1111));
    let mut consumer = create_stream_consumer(&rand_test_group(), None);
    consumer.subscribe(&vec![topic_name.as_str()]).unwrap();

    let _consumer_future = consumer.start()
        .take(100)
        .for_each(|message| {
            match message {
                Ok(m) => {
                    let id = message_map.get(&(m.partition(), m.offset())).unwrap();
                    assert_eq!(m.timestamp(), Timestamp::CreateTime(1111));
                    assert_eq!(m.payload_view::<str>().unwrap().unwrap(), value_fn(*id));
                    assert_eq!(m.key_view::<str>().unwrap().unwrap(), key_fn(*id));
                },
                Err(e) => panic!("Error receiving message: {:?}", e)
            };
            Ok(())
        })
        .wait();

    produce_messages(&topic_name, 10, &value_fn, &key_fn, Some(0), Some(999999));

    // Lookup the offsets
    let tpl = consumer.offsets_for_timestamp(999999, 100).unwrap();
    let tp = tpl.find_partition(&topic_name, 0).unwrap();
    assert_eq!(tp.offset(), Offset::Offset(100));
}

#[test]
fn test_consume_with_no_message_error() {
    let _r = env_logger::init();

    let mut consumer = create_stream_consumer(&rand_test_group(), None);

    let message_stream = consumer.start_with(Duration::from_millis(200), true);

    let mut first_poll_time = None;
    let mut timeouts_count = 0;
    for message in message_stream.wait() {
        match message {
            Ok(Err(KafkaError::NoMessageReceived)) => {
                // TODO: use entry interface for Options once available
                if first_poll_time.is_none() {
                    first_poll_time = Some(Instant::now());
                }
                timeouts_count += 1;
                if timeouts_count == 5 {
                    break;
                }
            }
            Ok(m) => panic!("A message was actually received: {:?}", m),
            Err(e) => panic!("Unexpected error while receiving message: {:?}", e)
        };
    }

    assert_eq!(timeouts_count, 5);
    // It should take 800ms
    assert!(Instant::now().duration_since(first_poll_time.unwrap()) < Duration::from_millis(1000));
    assert!(Instant::now().duration_since(first_poll_time.unwrap()) > Duration::from_millis(600));
}


// METADATA

#[test]
fn test_metadata() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    produce_messages(&topic_name, 1, &value_fn, &key_fn, Some(0), None);
    produce_messages(&topic_name, 1, &value_fn, &key_fn, Some(1), None);
    produce_messages(&topic_name, 1, &value_fn, &key_fn, Some(2), None);
    let consumer = create_stream_consumer(&rand_test_group(), None);

    let metadata = consumer.fetch_metadata(None, 5000).unwrap();

    let topic_metadata = metadata.topics().iter()
        .find(|m| m.name() == topic_name).unwrap();

    let mut ids = topic_metadata.partitions().iter().map(|p| p.id()).collect::<Vec<_>>();
    ids.sort();

    assert_eq!(ids, vec![0, 1, 2]);
    // assert_eq!(topic_metadata.error(), None);
    assert_eq!(topic_metadata.partitions().len(), 3);
    assert_eq!(topic_metadata.partitions()[0].leader(), 0);
    assert_eq!(topic_metadata.partitions()[1].leader(), 0);
    assert_eq!(topic_metadata.partitions()[2].leader(), 0);
    assert_eq!(topic_metadata.partitions()[0].replicas(), &[0]);
    assert_eq!(topic_metadata.partitions()[0].isr(), &[0]);

    let metadata_one_topic = consumer.fetch_metadata(Some(&topic_name), 5000).unwrap();
    assert_eq!(metadata_one_topic.topics().len(), 1);
}

// TODO: add check that commit cb gets called correctly
#[test]
fn test_consumer_commit_message() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    produce_messages(&topic_name, 10, &value_fn, &key_fn, Some(0), None);
    produce_messages(&topic_name, 11, &value_fn, &key_fn, Some(1), None);
    produce_messages(&topic_name, 12, &value_fn, &key_fn, Some(2), None);
    let mut consumer = create_stream_consumer(&rand_test_group(), None);
    consumer.subscribe(&vec![topic_name.as_str()]).unwrap();

    let _consumer_future = consumer.start()
        .take(33)
        .for_each(|message| {
            match message {
                Ok(m) => {
                    if m.partition() == 1 {
                        consumer.commit_message(&m, CommitMode::Async).unwrap();
                    }
                },
                Err(e) => panic!("Error receiving message: {:?}", e)
            };
            Ok(())
        })
        .wait();

    assert_eq!(consumer.fetch_watermarks(&topic_name, 0, 5000).unwrap(), (0, 10));
    assert_eq!(consumer.fetch_watermarks(&topic_name, 1, 5000).unwrap(), (0, 11));
    assert_eq!(consumer.fetch_watermarks(&topic_name, 2, 5000).unwrap(), (0, 12));

    let mut assignment = TopicPartitionList::new();
    assignment.add_partition_offset(&topic_name, 0, Offset::Invalid);
    assignment.add_partition_offset(&topic_name, 1, Offset::Invalid);
    assignment.add_partition_offset(&topic_name, 2, Offset::Invalid);
    assert_eq!(assignment, consumer.assignment().unwrap());

    let mut committed = TopicPartitionList::new();
    committed.add_partition_offset(&topic_name, 0, Offset::Invalid);
    committed.add_partition_offset(&topic_name, 1, Offset::Offset(11));
    committed.add_partition_offset(&topic_name, 2, Offset::Invalid);
    assert_eq!(committed, consumer.committed(5000).unwrap());

    let mut position = TopicPartitionList::new();
    position.add_partition_offset(&topic_name, 0, Offset::Offset(10));
    position.add_partition_offset(&topic_name, 1, Offset::Offset(11));
    position.add_partition_offset(&topic_name, 2, Offset::Offset(12));
    assert_eq!(position, consumer.position().unwrap());
}

#[test]
fn test_consumer_store_offset_commit() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    produce_messages(&topic_name, 10, &value_fn, &key_fn, Some(0), None);
    produce_messages(&topic_name, 11, &value_fn, &key_fn, Some(1), None);
    produce_messages(&topic_name, 12, &value_fn, &key_fn, Some(2), None);
    let mut config = HashMap::new();
    config.insert("enable.auto.offset.store", "false");
    let mut consumer = create_stream_consumer(&rand_test_group(), Some(config));
    consumer.subscribe(&vec![topic_name.as_str()]).unwrap();

    let _consumer_future = consumer.start()
        .take(33)
        .for_each(|message| {
            match message {
                Ok(m) => {
                    if m.partition() == 1 {
                        consumer.store_offset(&m).unwrap();
                    }
                },
                Err(e) => panic!("Error receiving message: {:?}", e)
            };
            Ok(())
        })
        .wait();

    // Commit the whole current state
    consumer.commit(None, CommitMode::Sync).unwrap();

    assert_eq!(consumer.fetch_watermarks(&topic_name, 0, 5000).unwrap(), (0, 10));
    assert_eq!(consumer.fetch_watermarks(&topic_name, 1, 5000).unwrap(), (0, 11));
    assert_eq!(consumer.fetch_watermarks(&topic_name, 2, 5000).unwrap(), (0, 12));

    let mut assignment = TopicPartitionList::new();
    assignment.add_partition_offset(&topic_name, 0, Offset::Invalid);
    assignment.add_partition_offset(&topic_name, 1, Offset::Invalid);
    assignment.add_partition_offset(&topic_name, 2, Offset::Invalid);
    assert_eq!(assignment, consumer.assignment().unwrap());

    let mut committed = TopicPartitionList::new();
    committed.add_partition_offset(&topic_name, 0, Offset::Invalid);
    committed.add_partition_offset(&topic_name, 1, Offset::Offset(11));
    committed.add_partition_offset(&topic_name, 2, Offset::Invalid);
    assert_eq!(committed, consumer.committed(5000).unwrap());

    let mut position = TopicPartitionList::new();
    position.add_partition_offset(&topic_name, 0, Offset::Offset(10));
    position.add_partition_offset(&topic_name, 1, Offset::Offset(11));
    position.add_partition_offset(&topic_name, 2, Offset::Offset(12));
    assert_eq!(position, consumer.position().unwrap());
}

#[test]
fn test_subscription() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    produce_messages(&topic_name, 10, &value_fn, &key_fn, None, None);
    let mut consumer = create_stream_consumer(&rand_test_group(), None);
    consumer.subscribe(&vec![topic_name.as_str()]).unwrap();

    let _consumer_future = consumer.start().take(10).wait();

    let mut tpl = TopicPartitionList::new();
    tpl.add_topic_unassigned(&topic_name);
    assert_eq!(tpl, consumer.subscription().unwrap());
}

#[test]
fn test_group_membership() {
    let _r = env_logger::init();

    let topic_name = rand_test_topic();
    let group_name = rand_test_group();
    produce_messages(&topic_name, 1, &value_fn, &key_fn, Some(0), None);
    produce_messages(&topic_name, 1, &value_fn, &key_fn, Some(1), None);
    produce_messages(&topic_name, 1, &value_fn, &key_fn, Some(2), None);
    let mut consumer = create_stream_consumer(&group_name, None);
    consumer.subscribe(&vec![topic_name.as_str()]).unwrap();

    // Make sure the consumer joins the group
    let _consumer_future = consumer.start()
        .take(1)
        .for_each(|_| Ok(()))
        .wait();

    let group_list = consumer.fetch_group_list(None, 5000).unwrap();

    // Print all the data, valgrind will check memory access
    for group in group_list.groups().iter() {
        println!("{} {} {} {}", group.name(), group.state(), group.protocol(), group.protocol_type());
        for member in group.members() {
            println!("  {} {} {}", member.id(), member.client_id(), member.client_host());
        }
    }

    let group_list2 = consumer.fetch_group_list(Some(&group_name), 5000).unwrap();
    assert_eq!(group_list2.groups().len(), 1);

    let consumer_group = group_list2.groups().iter().find(|&g| g.name() == group_name).unwrap();
    assert_eq!(consumer_group.members().len(), 1);

    let consumer_member = &consumer_group.members()[0];
    assert_eq!(consumer_member.client_id(), "rdkafka_integration_test_client");
}
