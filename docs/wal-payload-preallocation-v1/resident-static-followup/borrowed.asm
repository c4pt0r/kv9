
/mnt/data/kv9-work/batch-index-coalescing-development-20260915-first/fused/jemalloc-test-executable:     file format elf64-x86-64


Disassembly of section .text:

0000000000418ea0 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins>:
  418ea0:	push   %rbp
  418ea1:	push   %r15
  418ea3:	push   %r14
  418ea5:	push   %r13
  418ea7:	push   %r12
  418ea9:	push   %rbx
  418eaa:	sub    $0x48,%rsp
  418eae:	mov    %rdx,%r15
  418eb1:	mov    %rsi,%r13
  418eb4:	lea    0x8(%rdi),%rdx
  418eb8:	cmpl   $0x1,(%rdi)
  418ebb:	jne    418f63 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0xc3>
  418ec1:	mov    %ecx,0x44(%rsp)
  418ec5:	mov    (%rdx),%rax
  418ec8:	add    $0xfffffffffffffff8,%rax
  418ecc:	mov    %rdx,(%rsp)
  418ed0:	lea    0x8(%rsp),%r12
  418ed5:	mov    %rax,0x8(%rsp)
  418eda:	lea    0x1081f(%rip),%rax        # 429700 <triomphe::arc::Arc<T>::as_ptr>
  418ee1:	mov    %rax,0x10(%rsp)
  418ee6:	mov    %r12,%rdi
  418ee9:	call   393f50 <<archery::shared_pointer::kind::arct::ArcTK as archery::shared_pointer::kind::SharedPointerKind>::make_mut::{{closure}}>
  418eee:	mov    %rax,%rbx
  418ef1:	mov    %r12,%rdi
  418ef4:	call   *0x10(%rsp)
  418ef8:	mov    (%rsp),%rcx
  418efc:	mov    %rax,(%rcx)
  418eff:	mov    %rbx,0x38(%rsp)
  418f04:	mov    0x20(%rbx),%rbx
  418f08:	mov    0x8(%r13),%rbp
  418f0c:	mov    0x10(%r13),%r14
  418f10:	mov    0x8(%rbx),%rsi
  418f14:	mov    0x10(%rbx),%rdx
  418f18:	mov    %r14,%r12
  418f1b:	sub    %rdx,%r12
  418f1e:	cmovb  %r14,%rdx
  418f22:	mov    %rbp,%rdi
  418f25:	call   *0x539735(%rip)        # 952660 <memcmp@GLIBC_2.2.5>
  418f2b:	cltq
  418f2d:	test   %eax,%eax
  418f2f:	cmovne %rax,%r12
  418f33:	test   %r12,%r12
  418f36:	sets   %cl
  418f39:	setg   %al
  418f3c:	sub    %cl,%al
  418f3e:	je     4191e3 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x343>
  418f44:	movzbl %al,%eax
  418f47:	cmp    $0x1,%eax
  418f4a:	jne    41923e <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x39e>
  418f50:	mov    0x38(%rsp),%r14
  418f55:	mov    %r14,%rdi
  418f58:	add    $0x10,%rdi
  418f5c:	xor    %ebp,%ebp
  418f5e:	jmp    419248 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3a8>
  418f63:	mov    %rdi,%r14
  418f66:	mov    %rdx,0x38(%rsp)
  418f6b:	mov    0x8(%r13),%rbp
  418f6f:	mov    0x10(%r13),%r12
  418f73:	mov    $0x1,%ebx
  418f78:	mov    $0x1,%r13d
  418f7e:	test   %cl,%cl
  418f80:	je     41908d <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x1ed>
  418f86:	test   %r12,%r12
  418f89:	je     418fa0 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x100>
  418f8b:	mov    %r12,%rdi
  418f8e:	call   *0x537d5c(%rip)        # 950cf0 <_DYNAMIC+0x228>
  418f94:	mov    %rax,%r13
  418f97:	test   %rax,%rax
  418f9a:	je     4193f5 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x555>
  418fa0:	mov    %r13,%rdi
  418fa3:	mov    %rbp,%rsi
  418fa6:	mov    %r12,%rdx
  418fa9:	call   *0x537d49(%rip)        # 950cf8 <memcpy@GLIBC_2.14>
  418faf:	mov    0x8(%r15),%rbp
  418fb3:	mov    0x10(%r15),%r15
  418fb7:	test   %r15,%r15
  418fba:	je     418fd1 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x131>
  418fbc:	mov    %r15,%rdi
  418fbf:	call   *0x537d2b(%rip)        # 950cf0 <_DYNAMIC+0x228>
  418fc5:	mov    %rax,%rbx
  418fc8:	test   %rax,%rax
  418fcb:	je     419403 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x563>
  418fd1:	mov    %rbx,%rdi
  418fd4:	mov    %rbp,%rsi
  418fd7:	mov    %r15,%rdx
  418fda:	call   *0x537d18(%rip)        # 950cf8 <memcpy@GLIBC_2.14>
  418fe0:	movq   $0x1,(%rsp)
  418fe8:	mov    %r12,0x8(%rsp)
  418fed:	mov    %r13,0x10(%rsp)
  418ff2:	mov    %r12,0x18(%rsp)
  418ff7:	mov    %r15,0x20(%rsp)
  418ffc:	mov    %rbx,0x28(%rsp)
  419001:	mov    %r15,0x30(%rsp)
  419006:	mov    $0x38,%edi
  41900b:	call   *0x537cdf(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419011:	test   %rax,%rax
  419014:	je     41937c <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x4dc>
  41901a:	mov    0x30(%rsp),%rcx
  41901f:	mov    %rcx,0x30(%rax)
  419023:	movups (%rsp),%xmm0
  419027:	movups 0x10(%rsp),%xmm1
  41902c:	movups 0x20(%rsp),%xmm2
  419031:	movups %xmm2,0x20(%rax)
  419035:	movups %xmm1,0x10(%rax)
  419039:	movups %xmm0,(%rax)
  41903c:	add    $0x8,%rax
  419040:	movq   $0x1,(%rsp)
  419048:	movq   $0x0,0x8(%rsp)
  419051:	movq   $0x0,0x18(%rsp)
  41905a:	mov    %rax,0x28(%rsp)
  41905f:	movb   $0x1,0x30(%rsp)
  419064:	mov    $0x38,%edi
  419069:	call   *0x537c81(%rip)        # 950cf0 <_DYNAMIC+0x228>
  41906f:	test   %rax,%rax
  419072:	jne    41917f <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x2df>
  419078:	mov    $0x8,%edi
  41907d:	mov    $0x38,%esi
  419082:	call   *0x537cb0(%rip)        # 950d38 <_DYNAMIC+0x270>
  419088:	jmp    41943f <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x59f>
  41908d:	test   %r12,%r12
  419090:	je     4190a7 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x207>
  419092:	mov    %r12,%rdi
  419095:	call   *0x537c55(%rip)        # 950cf0 <_DYNAMIC+0x228>
  41909b:	mov    %rax,%r13
  41909e:	test   %rax,%rax
  4190a1:	je     4193f5 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x555>
  4190a7:	mov    %r13,%rdi
  4190aa:	mov    %rbp,%rsi
  4190ad:	mov    %r12,%rdx
  4190b0:	call   *0x537c42(%rip)        # 950cf8 <memcpy@GLIBC_2.14>
  4190b6:	mov    0x8(%r15),%rbp
  4190ba:	mov    0x10(%r15),%r15
  4190be:	test   %r15,%r15
  4190c1:	je     4190d8 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x238>
  4190c3:	mov    %r15,%rdi
  4190c6:	call   *0x537c24(%rip)        # 950cf0 <_DYNAMIC+0x228>
  4190cc:	mov    %rax,%rbx
  4190cf:	test   %rax,%rax
  4190d2:	je     419413 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x573>
  4190d8:	mov    %rbx,%rdi
  4190db:	mov    %rbp,%rsi
  4190de:	mov    %r15,%rdx
  4190e1:	call   *0x537c11(%rip)        # 950cf8 <memcpy@GLIBC_2.14>
  4190e7:	movq   $0x1,(%rsp)
  4190ef:	mov    %r12,0x8(%rsp)
  4190f4:	mov    %r13,0x10(%rsp)
  4190f9:	mov    %r12,0x18(%rsp)
  4190fe:	mov    %r15,0x20(%rsp)
  419103:	mov    %rbx,0x28(%rsp)
  419108:	mov    %r15,0x30(%rsp)
  41910d:	mov    $0x38,%edi
  419112:	call   *0x537bd8(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419118:	test   %rax,%rax
  41911b:	je     419391 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x4f1>
  419121:	mov    0x30(%rsp),%rcx
  419126:	mov    %rcx,0x30(%rax)
  41912a:	movups (%rsp),%xmm0
  41912e:	movups 0x10(%rsp),%xmm1
  419133:	movups 0x20(%rsp),%xmm2
  419138:	movups %xmm2,0x20(%rax)
  41913c:	movups %xmm1,0x10(%rax)
  419140:	movups %xmm0,(%rax)
  419143:	add    $0x8,%rax
  419147:	movq   $0x1,(%rsp)
  41914f:	movq   $0x0,0x8(%rsp)
  419158:	movq   $0x0,0x18(%rsp)
  419161:	mov    %rax,0x28(%rsp)
  419166:	movb   $0x0,0x30(%rsp)
  41916b:	mov    $0x38,%edi
  419170:	call   *0x537b7a(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419176:	test   %rax,%rax
  419179:	je     4193a6 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x506>
  41917f:	mov    %rax,%r15
  419182:	mov    0x30(%rsp),%rax
  419187:	mov    %rax,0x30(%r15)
  41918b:	movups (%rsp),%xmm0
  41918f:	movups 0x10(%rsp),%xmm1
  419194:	movups 0x20(%rsp),%xmm2
  419199:	movups %xmm2,0x20(%r15)
  41919e:	movups %xmm1,0x10(%r15)
  4191a3:	movups %xmm0,(%r15)
  4191a7:	add    $0x8,%r15
  4191ab:	cmpq   $0x0,(%r14)
  4191af:	mov    0x38(%rsp),%rax
  4191b4:	je     4191d0 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x330>
  4191b6:	mov    (%rax),%rax
  4191b9:	lea    -0x8(%rax),%rcx
  4191bd:	mov    %rcx,(%rsp)
  4191c1:	lock decq -0x8(%rax)
  4191c6:	jne    4191d0 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x330>
  4191c8:	mov    %rsp,%rdi
  4191cb:	call   429750 <triomphe::arc::Arc<T>::drop_slow>
  4191d0:	movq   $0x1,(%r14)
  4191d7:	mov    %r15,0x8(%r14)
  4191db:	mov    $0x1,%bpl
  4191de:	jmp    419270 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3d0>
  4191e3:	mov    -0x8(%rbx),%rax
  4191e7:	mov    0x38(%rsp),%rcx
  4191ec:	mov    %rbx,0x20(%rcx)
  4191f0:	cmp    $0x1,%rax
  4191f4:	jne    419281 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3e1>
  4191fa:	mov    0x8(%r15),%rsi
  4191fe:	mov    0x10(%r15),%r15
  419202:	movq   $0x0,0x28(%rbx)
  41920a:	cmp    0x18(%rbx),%r15
  41920e:	ja     4193bb <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x51b>
  419214:	xor    %r12d,%r12d
  419217:	mov    0x38(%rsp),%r14
  41921c:	mov    0x20(%rbx),%rdi
  419220:	add    %r12,%rdi
  419223:	mov    %r15,%rdx
  419226:	call   *0x537acc(%rip)        # 950cf8 <memcpy@GLIBC_2.14>
  41922c:	add    %r15,%r12
  41922f:	mov    %r12,0x28(%rbx)
  419233:	xor    %ebp,%ebp
  419235:	cmpb   $0x0,0x44(%rsp)
  41923a:	jne    41926b <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3cb>
  41923c:	jmp    419270 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3d0>
  41923e:	xor    %ebp,%ebp
  419240:	mov    0x38(%rsp),%r14
  419245:	mov    %r14,%rdi
  419248:	mov    %r13,%rsi
  41924b:	mov    %r15,%rdx
  41924e:	xor    %ecx,%ecx
  419250:	call   418ea0 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins>
  419255:	test   %al,%al
  419257:	je     419264 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3c4>
  419259:	mov    %r14,%rdi
  41925c:	call   41a5d0 <rpds::map::red_black_tree_map::Node<K,V,P>::balance>
  419261:	mov    $0x1,%bpl
  419264:	cmpb   $0x0,0x44(%rsp)
  419269:	je     419270 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3d0>
  41926b:	movb   $0x1,0x28(%r14)
  419270:	mov    %ebp,%eax
  419272:	add    $0x48,%rsp
  419276:	pop    %rbx
  419277:	pop    %r12
  419279:	pop    %r13
  41927b:	pop    %r14
  41927d:	pop    %r15
  41927f:	pop    %rbp
  419280:	ret
  419281:	mov    $0x1,%r13d
  419287:	mov    $0x1,%r12d
  41928d:	test   %r14,%r14
  419290:	je     4192a7 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x407>
  419292:	mov    %r14,%rdi
  419295:	call   *0x537a55(%rip)        # 950cf0 <_DYNAMIC+0x228>
  41929b:	mov    %rax,%r12
  41929e:	test   %rax,%rax
  4192a1:	je     419423 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x583>
  4192a7:	mov    %r12,%rdi
  4192aa:	mov    %rbp,%rsi
  4192ad:	mov    %r14,%rdx
  4192b0:	call   *0x537a42(%rip)        # 950cf8 <memcpy@GLIBC_2.14>
  4192b6:	mov    0x8(%r15),%rbp
  4192ba:	mov    0x10(%r15),%r15
  4192be:	test   %r15,%r15
  4192c1:	je     4192d8 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x438>
  4192c3:	mov    %r15,%rdi
  4192c6:	call   *0x537a24(%rip)        # 950cf0 <_DYNAMIC+0x228>
  4192cc:	mov    %rax,%r13
  4192cf:	test   %rax,%rax
  4192d2:	je     419431 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x591>
  4192d8:	mov    %r13,%rdi
  4192db:	mov    %rbp,%rsi
  4192de:	mov    %r15,%rdx
  4192e1:	call   *0x537a11(%rip)        # 950cf8 <memcpy@GLIBC_2.14>
  4192e7:	movq   $0x1,(%rsp)
  4192ef:	mov    %r14,0x8(%rsp)
  4192f4:	mov    %r12,0x10(%rsp)
  4192f9:	mov    %r14,0x18(%rsp)
  4192fe:	mov    %r15,0x20(%rsp)
  419303:	mov    %r13,0x28(%rsp)
  419308:	mov    %r15,0x30(%rsp)
  41930d:	mov    $0x38,%edi
  419312:	call   *0x5379d8(%rip)        # 950cf0 <_DYNAMIC+0x228>
  419318:	test   %rax,%rax
  41931b:	je     4193e3 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x543>
  419321:	mov    %rax,%r15
  419324:	mov    0x30(%rsp),%rax
  419329:	mov    %rax,0x30(%r15)
  41932d:	movups (%rsp),%xmm0
  419331:	movups 0x10(%rsp),%xmm1
  419336:	movups 0x20(%rsp),%xmm2
  41933b:	movups %xmm2,0x20(%r15)
  419340:	movups %xmm1,0x10(%r15)
  419345:	movups %xmm0,(%r15)
  419349:	add    $0x8,%r15
  41934d:	mov    0x38(%rsp),%r14
  419352:	mov    0x20(%r14),%rdi
  419356:	lock decq -0x8(%rdi)
  41935b:	jne    419366 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x4c6>
  41935d:	add    $0xfffffffffffffff8,%rdi
  419361:	call   429710 <triomphe::arc::Arc<T>::drop_slow>
  419366:	mov    %r15,0x20(%r14)
  41936a:	xor    %ebp,%ebp
  41936c:	cmpb   $0x0,0x44(%rsp)
  419371:	jne    41926b <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3cb>
  419377:	jmp    419270 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x3d0>
  41937c:	mov    $0x8,%edi
  419381:	mov    $0x38,%esi
  419386:	call   *0x5379ac(%rip)        # 950d38 <_DYNAMIC+0x270>
  41938c:	jmp    41943f <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x59f>
  419391:	mov    $0x8,%edi
  419396:	mov    $0x38,%esi
  41939b:	call   *0x537997(%rip)        # 950d38 <_DYNAMIC+0x270>
  4193a1:	jmp    41943f <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x59f>
  4193a6:	mov    $0x8,%edi
  4193ab:	mov    $0x38,%esi
  4193b0:	call   *0x537982(%rip)        # 950d38 <_DYNAMIC+0x270>
  4193b6:	jmp    41943f <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x59f>
  4193bb:	lea    0x18(%rbx),%rdi
  4193bf:	mov    $0x1,%ecx
  4193c4:	mov    $0x1,%r8d
  4193ca:	mov    %rsi,%r12
  4193cd:	xor    %esi,%esi
  4193cf:	mov    %r15,%rdx
  4193d2:	call   420440 <alloc::raw_vec::RawVecInner<A>::reserve::do_reserve_and_handle>
  4193d7:	mov    %r12,%rsi
  4193da:	mov    0x28(%rbx),%r12
  4193de:	jmp    419217 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x377>
  4193e3:	mov    $0x8,%edi
  4193e8:	mov    $0x38,%esi
  4193ed:	call   *0x537945(%rip)        # 950d38 <_DYNAMIC+0x270>
  4193f3:	jmp    41943f <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x59f>
  4193f5:	mov    $0x1,%edi
  4193fa:	mov    %r12,%rsi
  4193fd:	call   *0x5378cd(%rip)        # 950cd0 <_DYNAMIC+0x208>
  419403:	mov    $0x1,%edi
  419408:	mov    %r15,%rsi
  41940b:	call   *0x5378bf(%rip)        # 950cd0 <_DYNAMIC+0x208>
  419411:	jmp    41943f <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x59f>
  419413:	mov    $0x1,%edi
  419418:	mov    %r15,%rsi
  41941b:	call   *0x5378af(%rip)        # 950cd0 <_DYNAMIC+0x208>
  419421:	jmp    41943f <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x59f>
  419423:	mov    $0x1,%edi
  419428:	mov    %r14,%rsi
  41942b:	call   *0x53789f(%rip)        # 950cd0 <_DYNAMIC+0x208>
  419431:	mov    $0x1,%edi
  419436:	mov    %r15,%rsi
  419439:	call   *0x537891(%rip)        # 950cd0 <_DYNAMIC+0x208>
  41943f:	ud2
  419441:	mov    %rax,%rbx
  419444:	test   %r14,%r14
  419447:	je     4194d8 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x638>
  41944d:	mov    %r12,%rdi
  419450:	mov    %r14,%rsi
  419453:	xor    %edx,%edx
  419455:	call   *0x5378cd(%rip)        # 950d28 <_DYNAMIC+0x260>
  41945b:	mov    %rbx,%rdi
  41945e:	call   909540 <_Unwind_Resume@plt>
  419463:	mov    %rax,%rbx
  419466:	movq   $0x1,(%r14)
  41946d:	mov    %r15,0x8(%r14)
  419471:	mov    %rbx,%rdi
  419474:	call   909540 <_Unwind_Resume@plt>
  419479:	jmp    41947b <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x5db>
  41947b:	mov    %rax,%rbx
  41947e:	test   %r12,%r12
  419481:	je     4194d8 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x638>
  419483:	mov    %r13,%rdi
  419486:	mov    %r12,%rsi
  419489:	xor    %edx,%edx
  41948b:	call   *0x537897(%rip)        # 950d28 <_DYNAMIC+0x260>
  419491:	mov    %rbx,%rdi
  419494:	call   909540 <_Unwind_Resume@plt>
  419499:	mov    %rax,%rbx
  41949c:	mov    %r12,%rdi
  41949f:	call   *0x10(%rsp)
  4194a3:	mov    (%rsp),%rcx
  4194a7:	mov    %rax,(%rcx)
  4194aa:	mov    %rbx,%rdi
  4194ad:	call   909540 <_Unwind_Resume@plt>
  4194b2:	call   *0x537e20(%rip)        # 9512d8 <_DYNAMIC+0x810>
  4194b8:	jmp    4194e8 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x648>
  4194ba:	mov    %rax,%rbx
  4194bd:	mov    %rsp,%rdi
  4194c0:	call   40b980 <core::ptr::drop_in_place<triomphe::arc::ArcInner<rpds::map::red_black_tree_map::Node<alloc::vec::Vec<u8>,alloc::vec::Vec<u8>,archery::shared_pointer::kind::arct::ArcTK>>>>
  4194c5:	jmp    4194d8 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x638>
  4194c7:	call   *0x537e0b(%rip)        # 9512d8 <_DYNAMIC+0x810>
  4194cd:	mov    %rax,%rbx
  4194d0:	mov    %rsp,%rdi
  4194d3:	call   40b980 <core::ptr::drop_in_place<triomphe::arc::ArcInner<rpds::map::red_black_tree_map::Node<alloc::vec::Vec<u8>,alloc::vec::Vec<u8>,archery::shared_pointer::kind::arct::ArcTK>>>>
  4194d8:	mov    %rbx,%rdi
  4194db:	call   909540 <_Unwind_Resume@plt>
  4194e0:	call   *0x537df2(%rip)        # 9512d8 <_DYNAMIC+0x810>
  4194e6:	jmp    4194e8 <rpds::map::red_black_tree_map::Node<K,V,P>::insert_cloned::ins+0x648>
  4194e8:	mov    %rax,%rbx
  4194eb:	mov    %rsp,%rdi
  4194ee:	call   409590 <core::ptr::drop_in_place<triomphe::arc::ArcInner<rpds::map::entry::Entry<alloc::vec::Vec<u8>,alloc::vec::Vec<u8>>>>>
  4194f3:	mov    %rbx,%rdi
  4194f6:	call   909540 <_Unwind_Resume@plt>
