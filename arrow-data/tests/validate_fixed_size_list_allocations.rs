// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_buffer::{Buffer, NullBuffer};
use arrow_data::ArrayData;
use arrow_schema::{DataType, Field};
use std::sync::Arc;

struct TrackingAllocator;

static TRACKED_BYTES: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static TRACK_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK_ALLOCATIONS.try_with(Cell::get).unwrap_or(false) {
            TRACKED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if TRACK_ALLOCATIONS.try_with(Cell::get).unwrap_or(false) {
            TRACKED_BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[test]
fn fixed_size_list_validation_does_not_expand_parent_nulls_without_child_nulls() {
    // Expanding one parent null into this many child bits allocates a bitmap,
    // despite the child having no null bitmap to validate.
    let list_size = 1 << 20;
    let child = ArrayData::builder(DataType::Int8)
        .len(list_size)
        .add_buffer(Buffer::from(vec![0; list_size]))
        .build()
        .unwrap();
    let nulls = NullBuffer::new_null(1);

    TRACKED_BYTES.store(0, Ordering::Relaxed);
    TRACK_ALLOCATIONS.with(|track| track.set(true));
    let data = ArrayData::builder(DataType::FixedSizeList(
        Arc::new(Field::new("item", DataType::Int8, false)),
        list_size as i32,
    ))
    .len(1)
    .nulls(Some(nulls))
    .add_child_data(child)
    .build()
    .unwrap();
    let result = data.validate_nulls();
    TRACK_ALLOCATIONS.with(|track| track.set(false));

    result.unwrap();
    assert!(
        TRACKED_BYTES.load(Ordering::Relaxed) < list_size / 8,
        "validating a child with no null bitmap should not allocate the expanded parent bitmap"
    );
}
